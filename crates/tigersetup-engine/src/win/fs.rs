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
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Storage::FileSystem::{
    FlushFileBuffers, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
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
            let n = source
                .read(&mut buffer)
                .map_err(|err| Error::new("io_error", format!("cannot read source: {err}")))?;
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
