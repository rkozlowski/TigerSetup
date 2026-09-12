//! The TigerSetup builder: reads a developer's `TigerSetup.toml`, resolves the
//! file set, generates the runtime metadata and composes it with the engine
//! bytes into a single `Setup.exe`. Runs on the developer or CI machine; never
//! on the target, and never touches Windows mutation code.

pub mod builder;
pub mod dependencies;
pub mod fileset;
pub mod inspect;
pub mod manifest;
pub mod metadata;
pub mod resource;
pub mod winget;

use std::fmt;

pub use builder::{BuildRequest, BuildResult, build};
pub use tigersetup_format::payload::{Compression, PayloadStats};

/// A builder failure: a stable code and a human-readable message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildError {
    pub code: &'static str,
    pub message: String,
}

impl BuildError {
    pub fn new(code: &'static str, message: impl Into<String>) -> BuildError {
        BuildError {
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for BuildError {}

impl From<tigersetup_format::FormatError> for BuildError {
    fn from(err: tigersetup_format::FormatError) -> BuildError {
        BuildError {
            code: err.code,
            message: err.message,
        }
    }
}

impl From<std::io::Error> for BuildError {
    fn from(err: std::io::Error) -> BuildError {
        BuildError::new("io_error", err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, BuildError>;
