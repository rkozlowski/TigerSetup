//! The WinGet community catalog client shared by the builder (build-time
//! acquisition hints) and the engine (install-time refresh and download).
//!
//! The catalog is Microsoft-hosted and needs no client: three HTTPS reads
//! (`TigerSetup-Design.md` §7.9) — the pre-indexed SQLite source packaged as
//! a signed MSIX, a per-package compressed version list naming every
//! manifest with its SHA-256, and the merged manifest carrying
//! `InstallerUrl` and `InstallerSha256`. Each link is authenticated by the
//! one before it, as `winget.exe` authenticates them ([`winget`] has the
//! chain, [`source`] the package checks). Nothing here knows what any
//! particular package is; a requirement is a package identifier, a minimum
//! version, an architecture and a scope preference. HTTP goes through the
//! inbox WinHTTP and signatures through the inbox WinTrust, so no TLS or
//! certificate code is compiled in and the target machine never needs
//! `winget.exe`.

pub mod http;
pub mod manifest;
pub mod mszip;
pub mod source;
pub mod winget;

use std::fmt;

/// Why an acquisition could not be completed: the stable detail a run
/// reports beside `dependency_unacquirable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// Name resolution, connection or transport failure: the machine has no
    /// usable network path to the host.
    NetworkUnavailable,
    /// The server answered but the download did not complete or was refused
    /// (an HTTP error status, a protocol error, a local write failure).
    DownloadFailed,
    /// The bytes arrived but their SHA-256 is not the expected one.
    HashMismatch,
    /// A catalog document could not be fetched or understood.
    CatalogUnavailable,
    /// The catalog holds no version that satisfies the requirement for the
    /// architecture.
    NoCompatibleVersion,
    /// The caller's cancel flag was raised.
    Cancelled,
    /// The source package's signature does not verify, or its signer does
    /// not chain to a Microsoft application root.
    CatalogSignatureInvalid,
    /// The source package is signed but is not the WinGet community source:
    /// another name or publisher, or a version that is not a publication
    /// time.
    CatalogIdentityMismatch,
    /// The source index was published before the caller's bound: a stale or
    /// replayed index.
    CatalogIndexStale,
    /// A catalog document is not the one its authenticated parent describes:
    /// a source package entry that differs from its signed block map, a
    /// duplicate entry name, or a version list whose SHA-256 is not the
    /// index's.
    CatalogIntegrityFailure,
}

impl Reason {
    pub fn code(self) -> &'static str {
        match self {
            Reason::NetworkUnavailable => "network_unavailable",
            Reason::DownloadFailed => "download_failed",
            Reason::HashMismatch => "hash_mismatch",
            Reason::CatalogUnavailable => "catalog_unavailable",
            Reason::NoCompatibleVersion => "no_compatible_version",
            Reason::Cancelled => "cancelled",
            Reason::CatalogSignatureInvalid => "catalog_signature_invalid",
            Reason::CatalogIdentityMismatch => "catalog_identity_mismatch",
            Reason::CatalogIndexStale => "catalog_index_stale",
            Reason::CatalogIntegrityFailure => "catalog_integrity_failure",
        }
    }
}

/// A catalog or download failure: the stable reason and a human message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogError {
    pub reason: Reason,
    pub message: String,
}

impl CatalogError {
    pub fn new(reason: Reason, message: impl Into<String>) -> CatalogError {
        CatalogError {
            reason,
            message: message.into(),
        }
    }
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.reason.code(), self.message)
    }
}

impl std::error::Error for CatalogError {}

pub type Result<T> = std::result::Result<T, CatalogError>;
