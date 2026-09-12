//! The directory resource: created when absent, recorded as created only when
//! TigerSetup made it, removed only when empty.

use std::path::Path;

use crate::Result;
use crate::win::fs;

pub fn exists(path: &Path) -> bool {
    path.is_dir()
}

pub fn create(path: &Path) -> Result<()> {
    fs::create_directory(path)
}

/// Returns whether the directory is gone afterwards.
pub fn remove_if_empty(path: &Path) -> Result<bool> {
    fs::remove_directory_if_empty(path)
}
