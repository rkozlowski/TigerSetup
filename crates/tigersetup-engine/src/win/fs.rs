//! Durable file primitives. Two separate guarantees make a written file
//! durable: `FlushFileBuffers` on the temporary makes its *data* durable, and
//! `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)` makes the
//! *name change* durable. Everything the engine writes goes through here.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, DELETE, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_WRITE_DATA, FlushFileBuffers, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH, MoveFileExW, OPEN_EXISTING, REPLACEFILE_WRITE_THROUGH, ReplaceFileW,
};

use crate::{Error, Result};

/// The last Win32 error as text, for messages.
pub fn last_error() -> io::Error {
    io::Error::last_os_error()
}

/// Suffix of the temporary a file is written to before it is renamed over
/// its target.
pub const TEMP_SUFFIX: &str = ".tigersetup-new";

fn io_error(what: &str, path: &Path, err: impl std::fmt::Display) -> Error {
    Error::new("io_error", format!("{what} {}: {err}", path.display()))
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Forces the file's data and metadata to disk.
pub fn flush(file: &File, path: &Path) -> Result<()> {
    if unsafe { FlushFileBuffers(file.as_raw_handle() as HANDLE) } == 0 {
        return Err(io_error("cannot flush", path, last_error()));
    }
    Ok(())
}

/// Renames `from` over `to`, replacing an existing target, with the rename
/// itself written through to disk before the call returns.
pub fn rename_write_through(from: &Path, to: &Path) -> Result<()> {
    let from_w = wide(from);
    let to_w = wide(to);
    let ok = unsafe {
        MoveFileExW(
            from_w.as_ptr(),
            to_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if ok == 0 {
        return Err(Error::new(
            "io_error",
            format!(
                "cannot rename {} over {}: {}",
                from.display(),
                to.display(),
                last_error()
            ),
        ));
    }
    Ok(())
}

/// The temporary path for a target: `<target>.tigersetup-new`.
pub fn temp_path_for(target: &Path) -> PathBuf {
    let mut name = target
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(TEMP_SUFFIX);
    target.with_file_name(name)
}

/// A file being written next to its target. Dropping it before `commit`
/// removes the temporary.
pub struct Staged {
    target: PathBuf,
    temp: PathBuf,
    file: Option<File>,
    pub sha256: String,
    pub size: u64,
}

impl Staged {
    /// Writes everything `source` yields into the temporary, hashing on the
    /// way. Nothing is flushed yet.
    pub fn write(target: &Path, source: &mut dyn Read) -> Result<Staged> {
        let temp = temp_path_for(target);
        let file = File::create(&temp).map_err(|err| io_error("cannot create", &temp, err))?;
        let mut writer = io::BufWriter::with_capacity(256 * 1024, file);
        let mut hasher = Sha256::new();
        let mut buffer = vec![0u8; 256 * 1024];
        let mut size = 0u64;
        loop {
            // A source that is a payload entry reports its own code — a
            // CRC mismatch, a truncated stream — through the I/O error.
            let n = source.read(&mut buffer).map_err(|err| {
                let err = tigersetup_format::FormatError::from(err);
                match err.code {
                    "io_error" => {
                        Error::new("io_error", format!("cannot read source: {}", err.message))
                    }
                    _ => Error::from(err),
                }
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
            writer
                .write_all(&buffer[..n])
                .map_err(|err| io_error("cannot write", &temp, err))?;
            size += n as u64;
        }
        writer
            .flush()
            .map_err(|err| io_error("cannot write", &temp, err))?;
        let file = writer
            .into_inner()
            .map_err(|err| io_error("cannot write", &temp, err.error().to_string()))?;
        Ok(Staged {
            target: target.to_path_buf(),
            temp,
            file: Some(file),
            sha256: hex(&hasher.finalize()),
            size,
        })
    }

    /// `FlushFileBuffers` on the temporary: its data is now durable.
    pub fn flush(&mut self) -> Result<()> {
        match &self.file {
            Some(file) => flush(file, &self.temp),
            None => Ok(()),
        }
    }

    /// Closes the temporary and renames it over the target, write-through.
    pub fn commit(mut self) -> Result<()> {
        self.file = None;
        rename_write_through(&self.temp, &self.target)?;
        self.temp = PathBuf::new();
        Ok(())
    }

    /// Closes the temporary and puts it in the target's place, keeping the
    /// file that was there as `backup`: one `ReplaceFileW` where the backup
    /// lies on the target's volume, so the target is at every instant the
    /// old file or the new one and the old bytes move rather than being
    /// copied; a flushed copy to `backup` and a rename otherwise. A target
    /// that is not there, or a backup that already is (a repeated apply
    /// after a crash, whose backup is the original), is a plain rename.
    pub fn commit_with_backup(mut self, backup: &Path) -> Result<()> {
        self.file = None;
        if !self.target.exists() || backup.exists() {
            rename_write_through(&self.temp, &self.target)?;
            self.temp = PathBuf::new();
            return Ok(());
        }
        if let Some(parent) = backup.parent() {
            fs::create_dir_all(parent).map_err(|err| io_error("cannot create", parent, err))?;
        }
        let target_w = wide(&self.target);
        let temp_w = wide(&self.temp);
        let backup_w = wide(backup);
        let replaced = unsafe {
            ReplaceFileW(
                target_w.as_ptr(),
                temp_w.as_ptr(),
                backup_w.as_ptr(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if replaced == 0 {
            // Another volume, or a filesystem without the primitive: the
            // slow, always-correct way.
            copy_and_flush(&self.target, backup)?;
            rename_write_through(&self.temp, &self.target)?;
        }
        self.temp = PathBuf::new();
        Ok(())
    }
}

/// Moves `target` to `backup`: a rename where both lie on one volume, a
/// flushed copy and a delete otherwise. The bytes are then exactly where
/// the journal's undo record says they are, and nothing was copied on the
/// common path. An absent target is nothing to move (`Ok(false)`).
pub fn move_to_backup(target: &Path, backup: &Path) -> Result<bool> {
    if !target.exists() {
        return Ok(false);
    }
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error("cannot create", parent, err))?;
    }
    let target_w = wide(target);
    let backup_w = wide(backup);
    let moved = unsafe {
        MoveFileExW(
            target_w.as_ptr(),
            backup_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        copy_and_flush(target, backup)?;
        remove_file(target)?;
    }
    Ok(true)
}

/// Puts a backup back at `target`: a rename where the target is absent and
/// both lie on one volume — the backup then no longer exists, which a
/// repeated restore reads as already done — else a flushed copy that keeps
/// the backup.
pub fn restore_from_backup(backup: &Path, target: &Path) -> Result<()> {
    if !target.exists() {
        let backup_w = wide(backup);
        let target_w = wide(target);
        let moved =
            unsafe { MoveFileExW(backup_w.as_ptr(), target_w.as_ptr(), MOVEFILE_WRITE_THROUGH) };
        if moved != 0 {
            return Ok(());
        }
    }
    let mut file = File::open(backup).map_err(|err| {
        Error::new(
            "backup_missing",
            format!("cannot open backup {}: {err}", backup.display()),
        )
    })?;
    let mut staged = Staged::write(target, &mut file)?;
    staged.flush()?;
    staged.commit()
}

impl Drop for Staged {
    fn drop(&mut self) {
        self.file = None;
        if !self.temp.as_os_str().is_empty() {
            let _ = fs::remove_file(&self.temp);
        }
    }
}

/// Copies `source` to `backup` and flushes the copy, so that the undo data is
/// durable before anything destructive happens to `source`.
pub fn copy_and_flush(source: &Path, backup: &Path) -> Result<()> {
    if let Some(parent) = backup.parent() {
        fs::create_dir_all(parent).map_err(|err| io_error("cannot create", parent, err))?;
    }
    fs::copy(source, backup).map_err(|err| {
        Error::new(
            "io_error",
            format!(
                "cannot copy {} to {}: {err}",
                source.display(),
                backup.display()
            ),
        )
    })?;
    // FlushFileBuffers needs a handle with write access.
    let file = fs::OpenOptions::new()
        .write(true)
        .open(backup)
        .map_err(|err| io_error("cannot open", backup, err))?;
    flush(&file, backup)
}

/// What a target looks like right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inspection {
    Absent,
    Present { sha256: String, size: u64 },
}

/// Whether another process holds the file at `path` in a way an
/// installation must respect: an open for `DELETE | FILE_WRITE_DATA` access
/// with every sharing mode granted is refused with a sharing violation by
/// any holder that shares neither. Two kinds of holder matter and they
/// refuse different halves of it. A data file an application keeps open
/// (the usual `FILE_SHARE_READ` alone) refuses the delete, which is what
/// makes a replacement or a removal fail. The image of a running program —
/// its executable and every DLL it has loaded — is mapped by Windows with
/// `FILE_SHARE_READ | FILE_SHARE_DELETE`, so it can be renamed from under
/// the process and refuses only the write: a probe for delete access alone
/// would call a running application's own files free. The open asks for no
/// data and writes none, so a real-time scanner has no reason to read the
/// file for it — which is what keeps this probe cheap on files just written.
///
/// Where the write request is refused for a reason that is not a holder —
/// a read-only attribute, an access control list — the probe falls back to
/// the delete-access open alone, so such a file answers as it did before
/// the write was asked for. A file that is not there, or that cannot be
/// probed at all, answers `false`: a holder is a fact about a file that
/// exists, and an error here is one the mutation itself will report
/// properly.
pub fn is_held(path: &Path) -> bool {
    let wide = wide(path);
    match probe(&wide, DELETE | FILE_WRITE_DATA) {
        Probe::Held => true,
        Probe::Free => false,
        Probe::Refused => probe(&wide, DELETE) == Probe::Held,
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Probe {
    /// The open succeeded: nothing refuses this access.
    Free,
    /// `ERROR_SHARING_VIOLATION` (32) or `ERROR_LOCK_VIOLATION` (33): what a
    /// holder looks like.
    Held,
    /// Refused for another reason, which is not a holder.
    Refused,
}

fn probe(wide: &[u16], access: u32) -> Probe {
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let error = unsafe { windows_sys::Win32::Foundation::GetLastError() };
        return if matches!(error, 32 | 33) {
            Probe::Held
        } else {
            Probe::Refused
        };
    }
    unsafe { CloseHandle(handle) };
    Probe::Free
}

/// What a file's directory entry says about it: its size and its last-write
/// time (`FILETIME`). Two files with the same fingerprint that TigerSetup
/// wrote itself are the same file; a modification that keeps both is not
/// one a user makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fingerprint {
    pub size: u64,
    pub modified: i64,
}

/// The fingerprint of the file at `path`, or `None` when it is not there
/// (or is not a file).
pub fn fingerprint(path: &Path) -> Result<Option<Fingerprint>> {
    use std::os::windows::fs::MetadataExt;
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(Some(Fingerprint {
            size: metadata.len(),
            modified: metadata.last_write_time() as i64,
        })),
        Ok(_) => Ok(None),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io_error("cannot stat", path, err)),
    }
}

/// Inspects `path` the cheap way where it can: a file whose fingerprint is
/// exactly `recorded` is the file TigerSetup wrote and has `recorded_sha256`
/// without being read; any other file is hashed.
pub fn inspect_unless_unchanged(
    path: &Path,
    recorded: Option<Fingerprint>,
    recorded_sha256: &str,
) -> Result<Inspection> {
    if let Some(recorded) = recorded
        && let Some(current) = fingerprint(path)?
        && current == recorded
    {
        return Ok(Inspection::Present {
            sha256: recorded_sha256.to_string(),
            size: current.size,
        });
    }
    inspect(path)
}

/// Hashes the file at `path`, or reports it absent.
pub fn inspect(path: &Path) -> Result<Inspection> {
    match File::open(path) {
        Ok(mut file) => {
            let mut hasher = Sha256::new();
            let mut buffer = vec![0u8; 256 * 1024];
            let mut size = 0u64;
            loop {
                let n = file
                    .read(&mut buffer)
                    .map_err(|err| io_error("cannot read", path, err))?;
                if n == 0 {
                    break;
                }
                hasher.update(&buffer[..n]);
                size += n as u64;
            }
            Ok(Inspection::Present {
                sha256: hex(&hasher.finalize()),
                size,
            })
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Inspection::Absent),
        Err(err) => Err(io_error("cannot open", path, err)),
    }
}

/// Removes a file; an absent file is not an error.
pub fn remove_file(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(io_error("cannot delete", path, err)),
    }
}

pub fn create_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|err| io_error("cannot create directory", path, err))
}

/// Removes a directory only when it is empty; returns whether it was removed.
/// Never touches content TigerSetup did not put there.
pub fn remove_directory_if_empty(path: &Path) -> Result<bool> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(false),
        Err(err) => Err(io_error("cannot remove directory", path, err)),
    }
}

/// Deletes every `*.tigersetup-new` under `root`, returning what was removed.
pub fn sweep_temp_files(root: &Path) -> Vec<PathBuf> {
    let mut swept = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                pending.push(path);
            } else if path.to_string_lossy().ends_with(TEMP_SUFFIX)
                && fs::remove_file(&path).is_ok()
            {
                swept.push(path);
            }
        }
    }
    swept
}

fn hex(bytes: &[u8]) -> String {
    tigersetup_format::hex(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staged_write_replaces_target_durably() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a.txt");
        fs::write(&target, b"old").unwrap();
        let mut staged = Staged::write(&target, &mut &b"new content"[..]).unwrap();
        assert!(temp_path_for(&target).exists());
        assert_eq!(staged.size, 11);
        staged.flush().unwrap();
        staged.commit().unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new content");
        assert!(!temp_path_for(&target).exists());
        let Inspection::Present { sha256, size } = inspect(&target).unwrap() else {
            panic!("present")
        };
        assert_eq!(size, 11);
        assert_eq!(
            sha256,
            tigersetup_format::hex(&tigersetup_format::sha256(b"new content"))
        );
    }

    #[test]
    fn dropped_staging_removes_the_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("b.bin");
        let staged = Staged::write(&target, &mut &b"x"[..]).unwrap();
        drop(staged);
        assert!(!temp_path_for(&target).exists());
        assert_eq!(inspect(&target).unwrap(), Inspection::Absent);
    }

    #[test]
    fn sweep_finds_nested_temporaries_only() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("x/y")).unwrap();
        fs::write(dir.path().join("x/y/f.tigersetup-new"), b"").unwrap();
        fs::write(dir.path().join("x/keep.txt"), b"").unwrap();
        let swept = sweep_temp_files(dir.path());
        assert_eq!(swept.len(), 1);
        assert!(dir.path().join("x/keep.txt").exists());
    }

    #[test]
    fn backup_copy_and_conservative_directory_removal() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("s.bin");
        fs::write(&source, b"data").unwrap();
        let backup = dir.path().join("backup/1");
        copy_and_flush(&source, &backup).unwrap();
        assert_eq!(fs::read(&backup).unwrap(), b"data");
        assert!(!remove_directory_if_empty(&dir.path().join("backup")).unwrap());
        remove_file(&backup).unwrap();
        remove_file(&backup).unwrap();
        assert!(remove_directory_if_empty(&dir.path().join("backup")).unwrap());
    }
}

#[cfg(test)]
mod probe_tests {
    use super::*;
    use std::os::windows::fs::OpenOptionsExt;

    #[test]
    fn a_file_held_without_delete_sharing_is_held_and_one_shared_fully_is_not() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("held.txt");
        fs::write(&path, b"held").unwrap();
        assert!(!is_held(&path), "nobody holds it");
        {
            let _holder = fs::OpenOptions::new()
                .read(true)
                .share_mode(FILE_SHARE_READ)
                .open(&path)
                .unwrap();
            assert!(is_held(&path), "held without delete sharing");
        }
        {
            let _holder = fs::File::open(&path).unwrap();
            assert!(
                !is_held(&path),
                "std's default sharing allows a rename and a write"
            );
        }
        assert!(
            !is_held(&dir.path().join("absent.txt")),
            "absent is not held"
        );
    }

    /// A read-only file nobody holds refuses the write for its attribute,
    /// not for a holder, and answers as it did when only delete access was
    /// asked for.
    #[test]
    // The attribute is put back so the temporary directory can be removed.
    #[allow(clippy::permissions_set_readonly_false)]
    fn a_read_only_file_nobody_holds_is_not_held() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("readonly.txt");
        fs::write(&path, b"x").unwrap();
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&path, permissions).unwrap();
        assert!(!is_held(&path), "read-only is an attribute, not a holder");
        let mut permissions = fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(false);
        fs::set_permissions(&path, permissions).unwrap();
    }

    /// The image of a running program is held: Windows maps it with delete
    /// sharing, so it can be renamed from under the process, and a probe
    /// for delete access alone would call it free — which is how an upgrade
    /// once replaced a running application's files without ever asking the
    /// Restart Manager to close it.
    #[test]
    fn the_image_of_a_running_program_is_held_until_it_exits() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        let system = std::env::var_os("SystemRoot").expect("SystemRoot");
        let source = Path::new(&system).join("System32").join("cmd.exe");
        let image = dir.path().join("program.exe");
        fs::copy(&source, &image).unwrap();
        assert!(!is_held(&image), "a program nobody runs is not held");
        // `pause` reads a key from the piped, never-written standard input,
        // so the copy runs until it is killed.
        let mut child = Command::new(&image)
            .args(["/d", "/c", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("the copied program starts");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while !is_held(&image) {
            assert!(
                std::time::Instant::now() < deadline,
                "the running program's image was never seen as held"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!is_held(&image), "free again once the program has exited");
    }
}
