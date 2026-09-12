//! HTTP through the inbox WinHTTP (`winhttp.dll`, Windows 10 1809 and
//! Server 2019): the operating system's TLS and proxy configuration, no TLS
//! crate. Redirects are followed by WinHTTP itself (its default policy
//! refuses only HTTPS → HTTP), every phase has a bounded timeout, and a
//! response body streams to a sink with a progress callback and a cancel
//! flag, so a 250 MB runtime never sits in memory.
//!
//! Failures distinguish "no network path to the host" (name resolution,
//! connection, transport timeouts) from an HTTP failure the server did
//! answer with, because the engine reports them differently.

use std::ffi::c_void;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::GetLastError;
use windows_sys::Win32::Networking::WinHttp::{
    ERROR_WINHTTP_CANNOT_CONNECT, ERROR_WINHTTP_CONNECTION_ERROR, ERROR_WINHTTP_NAME_NOT_RESOLVED,
    ERROR_WINHTTP_TIMEOUT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_CONTENT_LENGTH, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};

use crate::{CatalogError, Reason};

/// The `User-Agent` every request carries.
pub const USER_AGENT: &str = "TigerSetup";

/// Timeouts in milliseconds: name resolution, connection, sending the
/// request, and waiting for each piece of the response.
const RESOLVE_TIMEOUT: i32 = 15_000;
const CONNECT_TIMEOUT: i32 = 15_000;
const SEND_TIMEOUT: i32 = 30_000;
const RECEIVE_TIMEOUT: i32 = 60_000;

const READ_CHUNK: usize = 64 * 1024;

/// A transport-level failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HttpError {
    /// No network path to the host: name resolution, connection or transport
    /// failure (the Win32 error code is kept for the log).
    Network(u32),
    /// WinHTTP failed for another reason (TLS, protocol, handle state).
    Protocol(u32),
    /// The server answered with a status other than 200.
    Status(u32),
    /// The URL could not be parsed.
    Url(String),
    /// The body exceeds the caller's limit.
    TooLarge(u64),
    /// Writing the body locally failed.
    Io(String),
    /// The caller's cancel flag was raised.
    Cancelled,
}

impl std::fmt::Display for HttpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpError::Network(code) => write!(f, "network unavailable (WinHTTP error {code})"),
            HttpError::Protocol(code) => write!(f, "WinHTTP error {code}"),
            HttpError::Status(status) => write!(f, "HTTP status {status}"),
            HttpError::Url(url) => write!(f, "invalid URL {url:?}"),
            HttpError::TooLarge(limit) => write!(f, "the response exceeds {limit} bytes"),
            HttpError::Io(message) => write!(f, "{message}"),
            HttpError::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl HttpError {
    /// The acquisition reason this failure maps to for a download.
    pub fn reason(&self) -> Reason {
        match self {
            HttpError::Network(_) => Reason::NetworkUnavailable,
            HttpError::Cancelled => Reason::Cancelled,
            _ => Reason::DownloadFailed,
        }
    }

    /// The acquisition reason this failure maps to for a catalog document.
    pub fn catalog_reason(&self) -> Reason {
        match self {
            HttpError::Network(_) => Reason::NetworkUnavailable,
            HttpError::Cancelled => Reason::Cancelled,
            _ => Reason::CatalogUnavailable,
        }
    }
}

/// A parsed `http://` or `https://` URL: what WinHTTP needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Url {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path with the query string, always starting with `/`.
    pub path: String,
}

impl Url {
    pub fn parse(url: &str) -> Result<Url, HttpError> {
        let invalid = || HttpError::Url(url.to_string());
        let (secure, rest) = if let Some(rest) = url.strip_prefix("https://") {
            (true, rest)
        } else if let Some(rest) = url.strip_prefix("http://") {
            (false, rest)
        } else {
            return Err(invalid());
        };
        let rest = rest.split('#').next().unwrap_or("");
        let (authority, path) = match rest.find('/') {
            Some(at) => (&rest[..at], &rest[at..]),
            None => (rest, "/"),
        };
        let authority = authority.rsplit('@').next().unwrap_or(authority);
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(']') || host.ends_with(']') => {
                (host, port.parse::<u16>().map_err(|_| invalid())?)
            }
            _ => (authority, if secure { 443 } else { 80 }),
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        if host.is_empty() {
            return Err(invalid());
        }
        Ok(Url {
            secure,
            host: host.to_string(),
            port,
            path: path.to_string(),
        })
    }

    /// The last path segment, for a local file name; empty when there is
    /// none.
    pub fn file_name(&self) -> &str {
        self.path
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("")
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error() -> u32 {
    unsafe { GetLastError() }
}

fn classify(code: u32) -> HttpError {
    match code {
        ERROR_WINHTTP_NAME_NOT_RESOLVED
        | ERROR_WINHTTP_CANNOT_CONNECT
        | ERROR_WINHTTP_TIMEOUT
        | ERROR_WINHTTP_CONNECTION_ERROR => HttpError::Network(code),
        other => HttpError::Protocol(other),
    }
}

fn open_session() -> Result<Handle, HttpError> {
    let agent = wide(USER_AGENT);
    let mut session = unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if session.is_null() {
        // Automatic proxy resolution needs Windows 8.1; the default proxy is
        // the fallback.
        session = unsafe {
            WinHttpOpen(
                agent.as_ptr(),
                WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
                std::ptr::null(),
                std::ptr::null(),
                0,
            )
        };
    }
    if session.is_null() {
        return Err(classify(last_error()));
    }
    let session = Handle(session);
    unsafe {
        WinHttpSetTimeouts(
            session.0,
            RESOLVE_TIMEOUT,
            CONNECT_TIMEOUT,
            SEND_TIMEOUT,
            RECEIVE_TIMEOUT,
        )
    };
    Ok(session)
}

struct Response {
    _connection: Handle,
    request: Handle,
    content_length: Option<u64>,
}

fn open(url: &Url) -> Result<(Handle, Response), HttpError> {
    let session = open_session()?;
    let host = wide(&url.host);
    let connection = unsafe { WinHttpConnect(session.0, host.as_ptr(), url.port, 0) };
    if connection.is_null() {
        return Err(classify(last_error()));
    }
    let connection = Handle(connection);
    let verb = wide("GET");
    let path = wide(&url.path);
    let request = unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            if url.secure { WINHTTP_FLAG_SECURE } else { 0 },
        )
    };
    if request.is_null() {
        return Err(classify(last_error()));
    }
    let request = Handle(request);
    let sent =
        unsafe { WinHttpSendRequest(request.0, std::ptr::null(), 0, std::ptr::null(), 0, 0, 0) };
    if sent == 0 {
        return Err(classify(last_error()));
    }
    if unsafe { WinHttpReceiveResponse(request.0, std::ptr::null_mut()) } == 0 {
        return Err(classify(last_error()));
    }
    let mut status: u32 = 0;
    let mut length = std::mem::size_of::<u32>() as u32;
    let ok = unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            std::ptr::null(),
            &mut status as *mut u32 as *mut c_void,
            &mut length,
            std::ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(classify(last_error()));
    }
    if status != 200 {
        return Err(HttpError::Status(status));
    }
    let mut content_length: u64 = 0;
    let mut length = std::mem::size_of::<u64>() as u32;
    let ok = unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_CONTENT_LENGTH | WINHTTP_QUERY_FLAG_NUMBER64,
            std::ptr::null(),
            &mut content_length as *mut u64 as *mut c_void,
            &mut length,
            std::ptr::null_mut(),
        )
    };
    let content_length = (ok != 0).then_some(content_length);
    Ok((
        session,
        Response {
            _connection: connection,
            request,
            content_length,
        },
    ))
}

/// `WINHTTP_QUERY_FLAG_NUMBER64`: the header as a 64-bit number.
const WINHTTP_QUERY_FLAG_NUMBER64: u32 = 0x0800_0000;

/// Performs a GET and streams the body to `sink`, reporting `(done, total)`
/// bytes (total 0 when the server sent no length) and stopping when
/// `cancel` is raised. Returns the number of bytes received.
pub fn get(
    url: &str,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), HttpError>,
    progress: &mut dyn FnMut(u64, u64),
    cancel: Option<&AtomicBool>,
) -> Result<u64, HttpError> {
    let url = Url::parse(url)?;
    let (_session, response) = open(&url)?;
    let total = response.content_length.unwrap_or(0);
    let mut buffer = vec![0u8; READ_CHUNK];
    let mut done: u64 = 0;
    progress(0, total);
    loop {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(HttpError::Cancelled);
        }
        let mut read: u32 = 0;
        let ok = unsafe {
            WinHttpReadData(
                response.request.0,
                buffer.as_mut_ptr() as *mut c_void,
                READ_CHUNK as u32,
                &mut read,
            )
        };
        if ok == 0 {
            return Err(classify(last_error()));
        }
        if read == 0 {
            break;
        }
        sink(&buffer[..read as usize])?;
        done += read as u64;
        progress(done, total);
    }
    Ok(done)
}

/// Fetches a small document into memory, refusing more than `max_len`
/// bytes.
pub fn fetch(url: &str, max_len: u64, cancel: Option<&AtomicBool>) -> Result<Vec<u8>, HttpError> {
    let mut bytes = Vec::new();
    get(
        url,
        &mut |chunk| {
            if bytes.len() as u64 + chunk.len() as u64 > max_len {
                return Err(HttpError::TooLarge(max_len));
            }
            bytes.extend_from_slice(chunk);
            Ok(())
        },
        &mut |_, _| {},
        cancel,
    )?;
    Ok(bytes)
}

/// What a completed download is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Downloaded {
    /// Lower-case hex SHA-256 of the file.
    pub sha256: String,
    pub length: u64,
}

/// Downloads `url` to `to`, hashing the bytes as they arrive. With an
/// expected SHA-256 (lower-case hex) the file is removed and
/// `hash_mismatch` reported when the bytes differ, so an unverified
/// download never survives on disk.
pub fn download(
    url: &str,
    to: &Path,
    expected_sha256: Option<&str>,
    progress: &mut dyn FnMut(u64, u64),
    cancel: Option<&AtomicBool>,
) -> crate::Result<Downloaded> {
    let io = |what: &str, err: std::io::Error| {
        CatalogError::new(
            Reason::DownloadFailed,
            format!("{what} {}: {err}", to.display()),
        )
    };
    if let Some(parent) = to.parent() {
        std::fs::create_dir_all(parent).map_err(|err| io("cannot create the directory of", err))?;
    }
    let mut file = File::create(to).map_err(|err| io("cannot create", err))?;
    let mut hasher = Sha256::new();
    let result = get(
        url,
        &mut |chunk| {
            hasher.update(chunk);
            file.write_all(chunk)
                .map_err(|err| HttpError::Io(format!("cannot write {}: {err}", to.display())))
        },
        progress,
        cancel,
    );
    let length = match result {
        Ok(length) => length,
        Err(err) => {
            drop(file);
            let _ = std::fs::remove_file(to);
            return Err(CatalogError::new(err.reason(), format!("GET {url}: {err}")));
        }
    };
    file.sync_all().map_err(|err| io("cannot flush", err))?;
    drop(file);
    let sha256 = tigersetup_format::hex(&hasher.finalize());
    if let Some(expected) = expected_sha256
        && !expected.eq_ignore_ascii_case(&sha256)
    {
        let _ = std::fs::remove_file(to);
        return Err(CatalogError::new(
            Reason::HashMismatch,
            format!("{url}: expected sha256 {expected}, received {sha256}"),
        ));
    }
    Ok(Downloaded { sha256, length })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read};
    use std::net::TcpListener;

    #[test]
    fn urls_are_parsed() {
        let url = Url::parse("https://cdn.winget.microsoft.com/cache/source2.msix").unwrap();
        assert_eq!(
            url,
            Url {
                secure: true,
                host: "cdn.winget.microsoft.com".into(),
                port: 443,
                path: "/cache/source2.msix".into()
            }
        );
        assert_eq!(url.file_name(), "source2.msix");
        let url = Url::parse("http://127.0.0.1:8080/a/b.exe?x=1#frag").unwrap();
        assert_eq!(url.port, 8080);
        assert_eq!(url.path, "/a/b.exe?x=1");
        assert_eq!(url.file_name(), "b.exe");
        assert_eq!(Url::parse("http://host").unwrap().path, "/");
        assert!(Url::parse("ftp://host/x").is_err());
        assert!(Url::parse("https:///x").is_err());
    }

    /// One-shot local server: answers the first request with `status` and
    /// `body`, records the request line.
    fn serve_once(
        status: &'static str,
        body: Vec<u8>,
    ) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
            }
            let mut stream = reader.into_inner();
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
            request_line
        });
        (format!("http://127.0.0.1:{port}"), handle)
    }

    #[test]
    fn download_streams_hashes_and_reports_progress() {
        let body: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let expected = tigersetup_format::hex(&tigersetup_format::sha256(&body));
        let (base, server) = serve_once("200 OK", body.clone());
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("deps").join("file.bin");
        let mut seen = Vec::new();
        let downloaded = download(
            &format!("{base}/file.bin"),
            &to,
            Some(&expected),
            &mut |done, total| seen.push((done, total)),
            None,
        )
        .unwrap();
        assert_eq!(downloaded.length, 200_000);
        assert_eq!(downloaded.sha256, expected);
        assert_eq!(std::fs::read(&to).unwrap(), body);
        assert_eq!(seen.first(), Some(&(0, 200_000)));
        assert_eq!(seen.last(), Some(&(200_000, 200_000)));
        assert!(server.join().unwrap().starts_with("GET /file.bin HTTP/1.1"));
    }

    #[test]
    fn hash_mismatch_removes_the_file() {
        let (base, server) = serve_once("200 OK", b"not the bytes".to_vec());
        let dir = tempfile::tempdir().unwrap();
        let to = dir.path().join("file.bin");
        let err = download(
            &format!("{base}/file.bin"),
            &to,
            Some(&"0".repeat(64)),
            &mut |_, _| {},
            None,
        )
        .unwrap_err();
        assert_eq!(err.reason, Reason::HashMismatch);
        assert!(!to.exists());
        server.join().unwrap();
    }

    #[test]
    fn http_errors_and_absent_servers_are_told_apart() {
        let (base, server) = serve_once("404 Not Found", b"nope".to_vec());
        let err = fetch(&format!("{base}/missing"), 1 << 20, None).unwrap_err();
        assert_eq!(err, HttpError::Status(404));
        assert_eq!(err.reason(), Reason::DownloadFailed);
        assert_eq!(err.catalog_reason(), Reason::CatalogUnavailable);
        server.join().unwrap();

        // A port nobody listens on: a connection failure, not an HTTP one.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let err = fetch(&format!("http://127.0.0.1:{port}/x"), 1 << 20, None).unwrap_err();
        assert!(matches!(err, HttpError::Network(_)), "{err:?}");
        assert_eq!(err.reason(), Reason::NetworkUnavailable);
    }

    #[test]
    fn cancel_and_size_limit_stop_a_transfer() {
        let (base, server) = serve_once("200 OK", vec![7u8; 300_000]);
        let err = fetch(&format!("{base}/big"), 100_000, None).unwrap_err();
        assert_eq!(err, HttpError::TooLarge(100_000));
        let _ = server.join();

        let (base, server) = serve_once("200 OK", vec![7u8; 10]);
        let cancel = AtomicBool::new(true);
        let mut sink = |_: &[u8]| Ok(());
        let err = get(
            &format!("{base}/x"),
            &mut sink,
            &mut |_, _| {},
            Some(&cancel),
        )
        .unwrap_err();
        assert_eq!(err, HttpError::Cancelled);
        let _ = server.join();
        let mut rest = Vec::new();
        let _ = std::io::empty().read_to_end(&mut rest);
    }
}
