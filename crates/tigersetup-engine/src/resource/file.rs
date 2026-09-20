//! The file resource: a payload entry installed at an install-relative path,
//! owned by its SHA-256.

use std::fs::File;
use std::path::Path;

use tigersetup_format::Payload;

use crate::win::fs::{self, Staged};
use crate::{Error, Result};

/// Opens the payload entry and writes it to the target's temporary. The
/// returned staging handle still has to be flushed and committed.
pub fn stage_from_payload(
    target: &Path,
    payload: &mut Payload,
    entry: &str,
    expected_size: Option<u64>,
) -> Result<Staged> {
    let mut source = payload.by_name(entry)?;
    let staged = Staged::write(target, &mut source)?;
    if let Some(expected) = expected_size
        && staged.size != expected
    {
        return Err(Error::new(
            "payload_entry_size_mismatch",
            format!(
                "{}: wrote {} bytes, metadata declares {expected}",
                target.display(),
                staged.size
            ),
        ));
    }
    Ok(staged)
}

/// Stages a copy of `source` (a backup) at the target's temporary.
pub fn stage_from_file(target: &Path, source: &Path) -> Result<Staged> {
    let mut file = File::open(source).map_err(|err| {
        Error::new(
            "backup_missing",
            format!("cannot open backup {}: {err}", source.display()),
        )
    })?;
    Staged::write(target, &mut file)
}

/// The SHA-256 a payload entry would have on disk, computed without writing.
pub fn payload_sha256(payload: &mut Payload, entry: &str) -> Result<String> {
    Ok(tigersetup_format::hex(&payload.sha256_of(entry)?))
}

/// Whether the file at `target` currently has `sha256`.
pub fn matches(target: &Path, sha256: &str) -> Result<bool> {
    Ok(
        matches!(fs::inspect(target)?, fs::Inspection::Present { sha256: actual, .. } if actual == sha256),
    )
}
