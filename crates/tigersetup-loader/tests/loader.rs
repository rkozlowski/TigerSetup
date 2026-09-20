//! The loader against synthetic installers: a copy of the built loader,
//! the fake engine compressed as the engine block, one-byte payload and
//! metadata blocks the loader never reads, and the format 3 footer.
//!
//! Every launch gets its own `%TEMP%` under Cargo's target directory, so
//! nothing here touches the real `%TEMP%\TigerSetup` or collides with a
//! real installer run. A failing package is always launched with at least
//! one argument: with none the loader reports in a message box, which no
//! test can dismiss.

use std::fs;
use std::io::Write;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, SystemTime};

use sha2::{Digest, Sha256};

const LOADER: &str = env!("TIGERSETUP_LOADER_EXE");
const FAKE_ENGINE: &str = env!("TIGERSETUP_FAKE_ENGINE_EXE");

const FOOTER_LEN: usize = 320;
const CRC_OFFSET: usize = 308;
const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

/// A test's own scratch space: the package directory and the `%TEMP%` the
/// loader is pointed at.
struct Scratch {
    root: PathBuf,
    temp: PathBuf,
    captures: AtomicU32,
}

impl Scratch {
    fn new(name: &str) -> Scratch {
        let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
            .join("loader")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let temp = root.join("temp");
        fs::create_dir_all(&temp).unwrap();
        Scratch {
            root,
            temp,
            captures: AtomicU32::new(0),
        }
    }

    fn next_capture(&self) -> u32 {
        self.captures.fetch_add(1, Ordering::Relaxed)
    }

    /// The root the loader extracts under.
    fn extraction_root(&self) -> PathBuf {
        self.temp.join("TigerSetup")
    }

    fn extraction_entries(&self) -> Vec<PathBuf> {
        match fs::read_dir(self.extraction_root()) {
            Ok(entries) => entries.map(|e| e.unwrap().path()).collect(),
            Err(_) => Vec::new(),
        }
    }

    fn command(&self, package: &Path) -> Command {
        let mut command = Command::new(package);
        command
            .env("TEMP", &self.temp)
            .env("TMP", &self.temp)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        command
    }
}

/// The footer's fields, encoded exactly as the format lays them out.
#[derive(Clone)]
struct Footer {
    format_major: u16,
    format_minor: u16,
    footer_length: u32,
    engine_offset: u64,
    engine_length: u64,
    engine_uncompressed_length: u64,
    payload_offset: u64,
    payload_length: u64,
    payload_uncompressed_length: u64,
    metadata_offset: u64,
    metadata_length: u64,
    metadata_uncompressed_length: u64,
    engine_sha256: [u8; 32],
    engine_executable_sha256: [u8; 32],
    payload_sha256: [u8; 32],
    metadata_block_sha256: [u8; 32],
    metadata_sha256: [u8; 32],
}

impl Footer {
    fn encode(&self) -> [u8; FOOTER_LEN] {
        let mut out = [0u8; FOOTER_LEN];
        out[0..8].copy_from_slice(b"TIGERSTP");
        out[8..10].copy_from_slice(&self.format_major.to_le_bytes());
        out[10..12].copy_from_slice(&self.format_minor.to_le_bytes());
        out[12..16].copy_from_slice(&self.footer_length.to_le_bytes());
        out[16..24].copy_from_slice(&self.engine_offset.to_le_bytes());
        out[24..32].copy_from_slice(&self.engine_length.to_le_bytes());
        out[32..40].copy_from_slice(&self.engine_uncompressed_length.to_le_bytes());
        out[40..48].copy_from_slice(&self.payload_offset.to_le_bytes());
        out[48..56].copy_from_slice(&self.payload_length.to_le_bytes());
        out[56..64].copy_from_slice(&self.payload_uncompressed_length.to_le_bytes());
        out[64..72].copy_from_slice(&self.metadata_offset.to_le_bytes());
        out[72..80].copy_from_slice(&self.metadata_length.to_le_bytes());
        out[80..88].copy_from_slice(&self.metadata_uncompressed_length.to_le_bytes());
        out[88..120].copy_from_slice(&self.engine_sha256);
        out[120..152].copy_from_slice(&self.engine_executable_sha256);
        out[152..184].copy_from_slice(&self.payload_sha256);
        out[184..216].copy_from_slice(&self.metadata_block_sha256);
        out[216..248].copy_from_slice(&self.metadata_sha256);
        let crc = crc32fast::hash(&out[..CRC_OFFSET]);
        out[CRC_OFFSET..CRC_OFFSET + 4].copy_from_slice(&crc.to_le_bytes());
        out[312..320].copy_from_slice(b"PTSREGIT");
        out
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

/// The blocks of a synthetic installer, before the footer.
struct Blocks {
    loader: Vec<u8>,
    engine: Vec<u8>,
    engine_uncompressed_length: u64,
    engine_executable_sha256: [u8; 32],
    payload: Vec<u8>,
    metadata: Vec<u8>,
}

/// The fake engine as one Zstandard frame: level 3, content size in the
/// header, no checksum, as the builder writes the engine block.
fn compress_engine(engine: &[u8]) -> Vec<u8> {
    let mut encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    encoder
        .set_pledged_src_size(Some(engine.len() as u64))
        .unwrap();
    encoder.include_checksum(false).unwrap();
    encoder.include_contentsize(true).unwrap();
    encoder.write_all(engine).unwrap();
    encoder.finish().unwrap()
}

fn blocks() -> Blocks {
    let engine = fs::read(FAKE_ENGINE).unwrap();
    Blocks {
        loader: fs::read(LOADER).unwrap(),
        engine_uncompressed_length: engine.len() as u64,
        engine_executable_sha256: sha256(&engine),
        engine: compress_engine(&engine),
        payload: vec![0x7e],
        metadata: vec![0x5a],
    }
}

/// The footer that describes `blocks` truthfully.
fn footer_for(blocks: &Blocks) -> Footer {
    let engine_offset = blocks.loader.len() as u64;
    let payload_offset = engine_offset + blocks.engine.len() as u64;
    let metadata_offset = payload_offset + blocks.payload.len() as u64;
    Footer {
        format_major: 3,
        format_minor: 0,
        footer_length: FOOTER_LEN as u32,
        engine_offset,
        engine_length: blocks.engine.len() as u64,
        engine_uncompressed_length: blocks.engine_uncompressed_length,
        payload_offset,
        payload_length: blocks.payload.len() as u64,
        payload_uncompressed_length: blocks.payload.len() as u64,
        metadata_offset,
        metadata_length: blocks.metadata.len() as u64,
        metadata_uncompressed_length: blocks.metadata.len() as u64,
        engine_sha256: sha256(&blocks.engine),
        engine_executable_sha256: blocks.engine_executable_sha256,
        payload_sha256: sha256(&blocks.payload),
        metadata_block_sha256: sha256(&blocks.metadata),
        metadata_sha256: sha256(&blocks.metadata),
    }
}

fn assemble(blocks: &Blocks, footer: &[u8]) -> Vec<u8> {
    let mut file = Vec::new();
    file.extend_from_slice(&blocks.loader);
    file.extend_from_slice(&blocks.engine);
    file.extend_from_slice(&blocks.payload);
    file.extend_from_slice(&blocks.metadata);
    file.extend_from_slice(footer);
    file
}

fn write_package(scratch: &Scratch, name: &str, bytes: &[u8]) -> PathBuf {
    let path = scratch.root.join(name);
    fs::write(&path, bytes).unwrap();
    path
}

/// A package the loader accepts.
fn valid_package(scratch: &Scratch, name: &str) -> PathBuf {
    let blocks = blocks();
    let footer = footer_for(&blocks);
    write_package(scratch, name, &assemble(&blocks, &footer.encode()))
}

/// What the fake engine reported.
struct Report {
    command_line: String,
    executable: PathBuf,
    elevated: bool,
    security: String,
}

fn parse_report(stdout: &str) -> Report {
    let field = |prefix: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .unwrap_or_else(|| panic!("the engine reported no {prefix:?} line in:\n{stdout}"))
            .to_string()
    };
    Report {
        command_line: field("command line: "),
        executable: PathBuf::from(field("executable: ")),
        elevated: field("elevated: ") == "1",
        security: field("security: "),
    }
}

fn utf8(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("the output is UTF-8")
}

/// Runs a package that must fail before anything is executed and returns
/// the message the loader reported.
fn failure_message(scratch: &Scratch, package: &Path) -> String {
    let output = scratch.command(package).arg("install").output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(2),
        "exit code of {}",
        package.display()
    );
    assert!(
        output.stdout.is_empty(),
        "nothing was executed: {}",
        utf8(&output.stdout)
    );
    let stderr = utf8(&output.stderr);
    let prefix = format!("error: {} cannot start: ", package.display());
    let message = stderr
        .strip_prefix(&prefix)
        .unwrap_or_else(|| panic!("stderr {stderr:?} does not start with {prefix:?}"));
    assert!(
        scratch.extraction_entries().is_empty(),
        "nothing is left under {}",
        scratch.extraction_root().display()
    );
    message.strip_suffix('\n').expect("one line").to_string()
}

fn successful_report(scratch: &Scratch, output: &Output) -> Report {
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        utf8(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "stderr: {}", utf8(&output.stderr));
    let report = parse_report(&utf8(&output.stdout));
    assert!(
        scratch.extraction_entries().is_empty(),
        "nothing is left under {}",
        scratch.extraction_root().display()
    );
    report
}

#[test]
fn a_valid_package_runs_its_engine_with_the_command_line_tail_and_the_package() {
    let scratch = Scratch::new("valid");
    let package = valid_package(&scratch, "Product Setup.exe");
    let output = scratch
        .command(&package)
        .raw_arg(r#"install --quiet "a b" zażółć"#)
        .output()
        .unwrap();
    let report = successful_report(&scratch, &output);

    let expected = format!(
        r#""{}" install --quiet "a b" zażółć --package "{}""#,
        report.executable.display(),
        package.display()
    );
    assert_eq!(report.command_line, expected);

    // The engine keeps the package's own name, in a directory of this
    // launch's own under the user's root.
    assert_eq!(report.executable.file_name().unwrap(), "Product Setup.exe");
    let directory = report.executable.parent().unwrap();
    assert_eq!(directory.parent().unwrap(), scratch.extraction_root());
    let name = directory.file_name().unwrap().to_str().unwrap();
    let parts: Vec<&str> = name.split('-').collect();
    assert_eq!(parts.len(), 3, "{name} is <pid>-<tick>-<attempt>");
    assert!(
        parts.iter().all(|part| part.parse::<u64>().is_ok()),
        "{name}"
    );
    assert_eq!(parts[2], "0");
}

#[test]
fn the_engines_exit_code_is_the_loaders() {
    let scratch = Scratch::new("exit-code");
    let package = valid_package(&scratch, "Setup.exe");
    for code in [7, 3010] {
        let output = scratch
            .command(&package)
            .args(["--exit", &code.to_string()])
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(code),
            "stderr: {}",
            utf8(&output.stderr)
        );
        parse_report(&utf8(&output.stdout));
        assert!(
            scratch.extraction_entries().is_empty(),
            "nothing is left after exit {code}"
        );
    }
}

#[test]
fn the_tail_is_blank_when_only_white_space_follows_the_program() {
    let scratch = Scratch::new("blank-tail");
    let package = valid_package(&scratch, "Setup.exe");
    // A blank tail is omitted, not forwarded as empty; the package is
    // valid, so no message box is at stake.
    let output = scratch.command(&package).raw_arg("   ").output().unwrap();
    let report = successful_report(&scratch, &output);
    assert_eq!(
        report.command_line,
        format!(
            r#""{}" --package "{}""#,
            report.executable.display(),
            package.display()
        )
    );
}

#[test]
fn the_program_token_is_skipped_by_the_c_runtime_rule() {
    let scratch = Scratch::new("program-token");
    let package = valid_package(&scratch, "Setup.exe");
    // The package path has no spaces, so it can be given unquoted, with a
    // tab as the separator, or quoted with several spaces after it.
    assert!(
        !package.to_str().unwrap().contains(' '),
        "{}",
        package.display()
    );
    let cases: [(&str, &str); 4] = [
        (r#""{package}" install --quiet"#, "install --quiet"),
        ("{package} install", "install"),
        ("{package}\tinstall\t--quiet", "install\t--quiet"),
        (
            r#""{package}"    --option "name"   "#,
            r#"--option "name"   "#,
        ),
    ];
    for (line, tail) in cases {
        let line = line.replace("{package}", package.to_str().unwrap());
        let output = spawn_raw(&scratch, &package, &line);
        let report = successful_report(&scratch, &output);
        assert_eq!(
            report.command_line,
            format!(
                r#""{}" {tail} --package "{}""#,
                report.executable.display(),
                package.display()
            ),
            "command line {line:?}"
        );
    }
}

/// Runs `package` with `line` as its exact command line, which
/// `std::process::Command` cannot produce: it always quotes the program.
/// Standard output and error go to a file the child inherits.
fn spawn_raw(scratch: &Scratch, package: &Path, line: &str) -> Output {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::{
        CloseHandle, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
    };
    use windows_sys::Win32::System::Threading::{
        CREATE_UNICODE_ENVIRONMENT, CreateProcessW, GetExitCodeProcess, INFINITE,
        PROCESS_INFORMATION, STARTF_USESTDHANDLES, STARTUPINFOW, WaitForSingleObject,
    };

    let wide = |text: &str| {
        text.encode_utf16()
            .chain(std::iter::once(0))
            .collect::<Vec<u16>>()
    };
    let capture = scratch
        .root
        .join(format!("capture-{}.txt", scratch.next_capture()));
    let file = fs::File::create(&capture).unwrap();
    let handle = file.as_raw_handle() as HANDLE;
    assert_ne!(
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) },
        0
    );

    // The test process's environment with the temporary directory
    // redirected, as a Unicode block.
    let mut environment: Vec<u16> = Vec::new();
    for (key, value) in std::env::vars_os() {
        let key = key.to_string_lossy();
        if key.eq_ignore_ascii_case("TEMP") || key.eq_ignore_ascii_case("TMP") {
            continue;
        }
        environment.extend(format!("{key}={}", value.to_string_lossy()).encode_utf16());
        environment.push(0);
    }
    for key in ["TEMP", "TMP"] {
        environment.extend(format!("{key}={}", scratch.temp.display()).encode_utf16());
        environment.push(0);
    }
    environment.push(0);

    let application = wide(package.to_str().unwrap());
    let mut command_line = wide(line);
    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.dwFlags = STARTF_USESTDHANDLES;
    startup.hStdInput = std::ptr::null_mut();
    startup.hStdOutput = handle;
    startup.hStdError = handle;
    let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
    let created = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            CREATE_UNICODE_ENVIRONMENT,
            environment.as_ptr() as *const std::ffi::c_void,
            std::ptr::null(),
            &startup,
            &mut process,
        )
    };
    assert_ne!(
        created,
        0,
        "CreateProcessW({line:?}): {}",
        std::io::Error::last_os_error()
    );
    let mut code = 0u32;
    unsafe {
        WaitForSingleObject(process.hProcess, INFINITE);
        GetExitCodeProcess(process.hProcess, &mut code);
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    drop(file);
    let stdout = fs::read(&capture).unwrap();
    Output {
        status: std::process::ExitStatus::from_raw(code),
        stdout,
        stderr: Vec::new(),
    }
}

#[test]
fn an_engine_that_does_not_match_its_hash_is_never_run() {
    let scratch = Scratch::new("engine-hash");
    let blocks = blocks();
    let mut footer = footer_for(&blocks);
    footer.engine_executable_sha256[0] ^= 0xff;
    let package = write_package(&scratch, "Setup.exe", &assemble(&blocks, &footer.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "the engine executable does not match the hash in the footer"
    );
}

#[test]
fn a_corrupted_engine_block_is_refused() {
    let scratch = Scratch::new("engine-block");
    let mut blocks = blocks();
    let middle = blocks.engine.len() / 2;
    blocks.engine[middle] ^= 0x55;

    // The block hash in the footer no longer matches: refused before the
    // decoder sees a byte.
    let mut footer = footer_for(&blocks);
    footer.engine_sha256 = sha256(&compress_engine(&fs::read(FAKE_ENGINE).unwrap()));
    let package = write_package(&scratch, "Setup.exe", &assemble(&blocks, &footer.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "the compressed engine block does not match the hash in the footer"
    );

    // With the block hash recomputed over the corrupted bytes, the frame
    // header is what the decoder refuses.
    blocks.engine[middle] ^= 0x55;
    blocks.engine[0] ^= 0xff;
    let footer = footer_for(&blocks);
    let package = write_package(&scratch, "Setup2.exe", &assemble(&blocks, &footer.encode()));
    let message = failure_message(&scratch, &package);
    assert!(
        message.starts_with("the engine block cannot be decompressed"),
        "{message}"
    );

    // A byte flipped inside the frame, hash recomputed: whatever the decoder
    // makes of it, nothing is executed and nothing is left.
    blocks.engine[0] ^= 0xff;
    blocks.engine[middle] ^= 0x55;
    let footer = footer_for(&blocks);
    let package = write_package(&scratch, "Setup3.exe", &assemble(&blocks, &footer.encode()));
    let message = failure_message(&scratch, &package);
    assert!(
        message.starts_with("the engine block cannot be decompressed")
            || message == "the engine executable does not match the hash in the footer"
            || message.starts_with("the engine block decompresses to"),
        "{message}"
    );
}

#[test]
fn an_engine_longer_than_declared_is_refused() {
    let scratch = Scratch::new("engine-length");
    let blocks = blocks();
    let mut footer = footer_for(&blocks);
    footer.engine_uncompressed_length -= 1;
    let package = write_package(&scratch, "Setup.exe", &assemble(&blocks, &footer.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "the engine block decompresses to more than the footer declares"
    );
    let mut footer = footer_for(&blocks);
    footer.engine_uncompressed_length += 1;
    let package = write_package(&scratch, "Setup2.exe", &assemble(&blocks, &footer.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        format!(
            "the engine block decompresses to {} bytes, the footer declares {}",
            blocks.engine_uncompressed_length,
            blocks.engine_uncompressed_length + 1
        )
    );
}

#[test]
fn a_malformed_footer_is_refused() {
    let scratch = Scratch::new("footer");
    let blocks = blocks();
    let footer = footer_for(&blocks);

    let mut bytes = footer.encode();
    bytes[0..8].copy_from_slice(b"NOTMAGIC");
    let package = write_package(&scratch, "magic.exe", &assemble(&blocks, &bytes));
    assert_eq!(
        failure_message(&scratch, &package),
        "the file does not end with a TigerSetup footer"
    );

    let mut bytes = footer.encode();
    bytes[20] ^= 0xff;
    let actual = crc32fast::hash(&bytes[..CRC_OFFSET]);
    let expected = u32::from_le_bytes(bytes[CRC_OFFSET..CRC_OFFSET + 4].try_into().unwrap());
    let package = write_package(&scratch, "crc.exe", &assemble(&blocks, &bytes));
    assert_eq!(
        failure_message(&scratch, &package),
        format!("footer CRC-32 is {actual:08x}, expected {expected:08x}")
    );

    let mut other = footer.clone();
    other.format_major = 2;
    let package = write_package(&scratch, "major.exe", &assemble(&blocks, &other.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "installer format 2.0 is not supported by format 3.0"
    );

    let mut other = footer.clone();
    other.footer_length = 256;
    let package = write_package(&scratch, "length.exe", &assemble(&blocks, &other.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "footer length field is 256, expected 320"
    );

    let maps = [
        |f: &mut Footer| f.engine_offset += 1,
        |f: &mut Footer| f.engine_length += 1,
        |f: &mut Footer| f.engine_uncompressed_length = 0,
        |f: &mut Footer| f.metadata_length = 0,
        |f: &mut Footer| f.metadata_uncompressed_length = 0,
        |f: &mut Footer| f.payload_uncompressed_length = 0,
        |f: &mut Footer| f.engine_offset = u64::MAX,
    ];
    for (index, map) in maps.iter().enumerate() {
        let mut other = footer.clone();
        map(&mut other);
        let name = format!("map{index}.exe");
        let package = write_package(&scratch, &name, &assemble(&blocks, &other.encode()));
        assert_eq!(
            failure_message(&scratch, &package),
            "the footer's block map does not describe the file",
            "{name}"
        );
    }
}

#[test]
fn a_truncated_package_has_no_footer() {
    let scratch = Scratch::new("truncated");
    let blocks = blocks();
    let footer = footer_for(&blocks);
    let mut bytes = assemble(&blocks, &footer.encode());
    bytes.truncate(bytes.len() - 100);
    let package = write_package(&scratch, "Setup.exe", &bytes);
    assert_eq!(
        failure_message(&scratch, &package),
        "the file does not end with a TigerSetup footer"
    );
}

/// The offset of the security data directory in the loader's PE headers.
fn security_directory_offset(pe: &[u8]) -> usize {
    assert_eq!(&pe[0..2], b"MZ");
    let pe_offset = u32::from_le_bytes(pe[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&pe[pe_offset..pe_offset + 4], b"PE\0\0");
    let optional = pe_offset + 24;
    assert_eq!(
        u16::from_le_bytes(pe[optional..optional + 2].try_into().unwrap()),
        0x20b
    );
    optional + 112 + 4 * 8
}

#[test]
fn a_signed_package_carries_its_footer_before_the_certificate_table() {
    let scratch = Scratch::new("signed");
    let mut blocks = blocks();
    let footer = footer_for(&blocks);
    let mut bytes = assemble(&blocks, &footer.encode());
    // Pretend a certificate table follows the footer: the security
    // directory names it, and the loader finds the footer before it.
    let certificate_offset = bytes.len() as u32;
    let certificate = vec![0xccu8; 500];
    bytes.extend_from_slice(&certificate);
    let at = security_directory_offset(&blocks.loader);
    bytes[at..at + 4].copy_from_slice(&certificate_offset.to_le_bytes());
    bytes[at + 4..at + 8].copy_from_slice(&(certificate.len() as u32).to_le_bytes());
    let package = write_package(&scratch, "Signed.exe", &bytes);
    let output = scratch.command(&package).arg("install").output().unwrap();
    let report = successful_report(&scratch, &output);
    assert!(
        report
            .command_line
            .ends_with(&format!(r#"--package "{}""#, package.display()))
    );

    // A security directory that names a table too short to leave room for
    // a footer before it is reported as such.
    let at = security_directory_offset(&blocks.loader);
    blocks.loader[at..at + 4].copy_from_slice(&100u32.to_le_bytes());
    blocks.loader[at + 4..at + 8].copy_from_slice(&1u32.to_le_bytes());
    let package = write_package(&scratch, "Short.exe", &assemble(&blocks, &footer.encode()));
    assert_eq!(
        failure_message(&scratch, &package),
        "the file is shorter than a footer"
    );
}

#[test]
fn concurrent_launches_keep_their_own_directories() {
    let scratch = Scratch::new("concurrent");
    let package = valid_package(&scratch, "Setup.exe");
    let children: Vec<_> = (0..8)
        .map(|_| {
            scratch
                .command(&package)
                .args(["--hold", "800"])
                .spawn()
                .unwrap()
        })
        .collect();
    let outputs: Vec<Output> = children
        .into_iter()
        .map(|child| child.wait_with_output().unwrap())
        .collect();
    let mut executables = Vec::new();
    for output in &outputs {
        assert_eq!(
            output.status.code(),
            Some(0),
            "stderr: {}",
            utf8(&output.stderr)
        );
        let report = parse_report(&utf8(&output.stdout));
        executables.push(report.executable);
    }
    executables.sort();
    executables.dedup();
    assert_eq!(executables.len(), 8, "every launch had its own directory");
    assert!(
        scratch.extraction_entries().is_empty(),
        "every launch removed its own"
    );
}

/// Sets the last-write time of a directory.
fn set_modified(directory: &Path, when: SystemTime) {
    let handle = fs::OpenOptions::new()
        .write(true)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(directory)
        .unwrap();
    handle.set_modified(when).unwrap();
}

#[test]
fn stale_extraction_directories_are_swept_and_young_ones_kept() {
    let scratch = Scratch::new("sweep");
    let root = scratch.extraction_root();
    let old = root.join("1-1-0");
    let young = root.join("2-2-0");
    let file = root.join("not-a-directory.txt");
    fs::create_dir_all(&old).unwrap();
    fs::write(old.join("Setup.exe"), b"left behind").unwrap();
    fs::create_dir_all(&young).unwrap();
    fs::write(&file, b"a file is not swept").unwrap();
    set_modified(&old, SystemTime::now() - Duration::from_secs(25 * 60 * 60));
    set_modified(&young, SystemTime::now() - Duration::from_secs(60 * 60));
    set_modified(&root, SystemTime::now() - Duration::from_secs(48 * 60 * 60));

    let package = valid_package(&scratch, "Setup.exe");
    let output = scratch.command(&package).arg("install").output().unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        utf8(&output.stderr)
    );

    assert!(!old.exists(), "the day-old directory is swept");
    assert!(young.is_dir(), "the hour-old directory is kept");
    assert!(file.is_file(), "a file is not a launch's directory");
}

#[test]
fn nothing_is_left_behind_after_a_failed_extraction() {
    let scratch = Scratch::new("cleanup");
    let blocks = blocks();
    let mut footer = footer_for(&blocks);
    footer.engine_executable_sha256[31] ^= 0x01;
    let package = write_package(&scratch, "Setup.exe", &assemble(&blocks, &footer.encode()));
    let output = scratch.command(&package).arg("install").output().unwrap();
    assert_eq!(output.status.code(), Some(2));
    // The root exists, because the directory was created before the
    // engine was checked, and holds nothing: no directory, no `.partial`.
    assert!(scratch.extraction_root().is_dir());
    assert!(scratch.extraction_entries().is_empty());
}

#[test]
fn the_extraction_root_is_created_with_its_parents() {
    let scratch = Scratch::new("parents");
    let package = valid_package(&scratch, "Setup.exe");
    // `%TEMP%` itself does not exist yet: the root and everything above it
    // is created, as the engine's own directory then is.
    let temp = scratch.root.join("missing").join("temp");
    assert!(!temp.exists());
    let output = scratch
        .command(&package)
        .env("TEMP", &temp)
        .env("TMP", &temp)
        .arg("install")
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        utf8(&output.stderr)
    );
    let report = parse_report(&utf8(&output.stdout));
    assert!(report.executable.starts_with(temp.join("TigerSetup")));
    assert_eq!(fs::read_dir(temp.join("TigerSetup")).unwrap().count(), 0);
}

#[test]
fn an_unelevated_launch_extracts_under_the_users_temp() {
    let scratch = Scratch::new("unelevated");
    let package = valid_package(&scratch, "Setup.exe");
    let output = scratch.command(&package).arg("install").output().unwrap();
    let report = successful_report(&scratch, &output);
    if report.elevated {
        eprintln!("the test process is elevated; the unelevated root is not exercised");
        return;
    }
    assert!(
        report.executable.starts_with(scratch.extraction_root()),
        "{} lies under {}",
        report.executable.display(),
        scratch.extraction_root().display()
    );
}

/// An elevated launch extracts under `%SystemRoot%\Temp`, into a
/// directory that grants SYSTEM and Administrators alone. This test needs
/// an elevated test process and is ignored otherwise; the lab's elevation
/// rows, which drive the real UAC handoff, are the proof that counts. Run
/// it from an elevated prompt with `cargo test -- --ignored`.
#[test]
#[ignore]
fn an_elevated_launch_extracts_under_a_protected_system_directory() {
    let scratch = Scratch::new("elevated");
    let package = valid_package(&scratch, "Setup.exe");
    let output = scratch.command(&package).arg("install").output().unwrap();
    let report = successful_report(&scratch, &output);
    assert!(report.elevated, "this test must run elevated");

    let windows = PathBuf::from(std::env::var_os("SystemRoot").unwrap());
    let directory = report.executable.parent().unwrap();
    assert_eq!(directory.parent().unwrap(), windows.join("Temp"));
    let name = directory.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("TigerSetup-"), "{name}");
    assert!(!directory.exists(), "the directory is removed");

    // Owner Administrators; a protected list of exactly two allow entries,
    // each inheritable by files and folders and granting full control:
    // SYSTEM and Administrators. Rights compared as numbers.
    let fields: Vec<&str> = report.security.split(' ').collect();
    assert!(
        fields.contains(&"owner=S-1-5-32-544"),
        "{}",
        report.security
    );
    assert!(fields.contains(&"protected=1"), "{}", report.security);
    let mut aces: Vec<&str> = fields
        .iter()
        .filter_map(|field| field.strip_prefix("ace="))
        .collect();
    aces.sort();
    assert_eq!(
        aces,
        [
            "00000000:00000003:001f01ff:S-1-5-18",
            "00000000:00000003:001f01ff:S-1-5-32-544"
        ],
        "{}",
        report.security
    );
}
