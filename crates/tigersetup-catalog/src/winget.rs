//! Resolving a requirement against the WinGet community source:
//!
//! ```text
//! GET cache/source2.msix                                  the index (an MSIX; Public/index.db inside)
//!     Microsoft signature, identity, publication bound, block map (crate::source)
//!     packages.hash for the identifier: SHA-256 of the version data
//! GET cache/packages/<id>/<hash[0:8]>/versionData.mszyml  verified against packages.hash
//!     every version, its manifest path and SHA-256; highest satisfying the requirement
//! GET cache/<rP>                                          the merged manifest, verified against s256H
//!     the installer entry for the architecture and scope preference
//! ```
//!
//! The chain is what `winget.exe` itself reads, authenticated the way it
//! authenticates it: the index by Microsoft's signature, and every later
//! document by the hash its authenticated parent lists, down to the
//! installer, which the download verifies against `InstallerSha256`. A
//! network attacker, a compromised CDN edge or a replayed old index cannot
//! change what is installed. What the chain cannot catch is a bad entry in
//! the catalog itself — a malicious publisher's package, or a bad merge in
//! `winget-pkgs`: Microsoft signs the index it published, whatever was
//! merged into it.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use rusqlite::{Connection, OpenFlags, OptionalExtension};
use tigersetup_format::identity::{Scope, compare_versions, version_satisfies};
use yaml_rust2::YamlLoader;

use crate::manifest::{self, InstallerEntry, MergedManifest};
use crate::source::{self, SourceContent, SourceIdentity};
use crate::{CatalogError, Reason, Result, http, mszip};

/// Where the community source is served from.
pub const CATALOG_BASE: &str = "https://cdn.winget.microsoft.com/cache/";
/// The pre-indexed source package.
pub const INDEX_URL: &str = "https://cdn.winget.microsoft.com/cache/source2.msix";
/// The index database inside the MSIX.
pub const INDEX_ENTRY: &str = "Public/index.db";

const INDEX_MAX_LEN: u64 = 256 << 20;
const DOCUMENT_MAX_LEN: u64 = 16 << 20;
const VERSION_DATA_MAX_LEN: usize = 16 << 20;
/// How many candidate versions' manifests are tried before giving up.
const MANIFEST_ATTEMPTS: usize = 3;

/// How a resolution judges the source index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolveOptions {
    /// The oldest index publication accepted, in Unix seconds: an index
    /// published earlier is refused with `catalog_index_stale`, so a
    /// replayed old index cannot pin resolution to old metadata.
    pub index_not_before: i64,
}

impl ResolveOptions {
    /// The default bound: an index published within the last 30 days.
    pub const DEFAULT_MAX_INDEX_AGE_SECONDS: i64 = 30 * 86_400;

    /// Accepts an index published no earlier than `not_before` (Unix
    /// seconds).
    pub fn index_not_before(not_before: i64) -> ResolveOptions {
        ResolveOptions {
            index_not_before: not_before,
        }
    }
}

impl Default for ResolveOptions {
    /// An index published within [`Self::DEFAULT_MAX_INDEX_AGE_SECONDS`] of
    /// this machine's clock.
    fn default() -> ResolveOptions {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        ResolveOptions::index_not_before(now - Self::DEFAULT_MAX_INDEX_AGE_SECONDS)
    }
}

/// What a dependency requires of the catalog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement<'a> {
    /// Same major, not lower; empty means any version.
    pub minimum_version: &'a str,
    /// `x64`.
    pub architecture: &'a str,
    /// `user` when the product installs per user, else `machine`.
    pub scope: Scope,
}

/// The package row the index holds for an identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRow {
    pub id: String,
    pub latest_version: String,
    /// `packages.hash` in lower-case hex: the SHA-256 of the compressed
    /// version data document.
    pub hash: String,
    /// Its first eight hex digits: the version data's path segment.
    pub hash_prefix: String,
}

/// One version of a package as the version data lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionEntry {
    pub version: String,
    /// The merged manifest's path under [`CATALOG_BASE`].
    pub relative_path: String,
    /// Lower-case hex SHA-256 of the manifest document.
    pub sha256: String,
}

/// What resolution found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The source index resolution read: `Identity/@Version` of the
    /// authenticated package.
    pub index_version: String,
    pub version: String,
    pub manifest_path: String,
    pub installer: InstallerEntry,
}

fn catalog_get(url: &str, max_len: u64, cancel: Option<&AtomicBool>) -> Result<Vec<u8>> {
    http::fetch(url, max_len, cancel)
        .map_err(|err| CatalogError::new(err.catalog_reason(), format!("GET {url}: {err}")))
}

fn unavailable(message: impl Into<String>) -> CatalogError {
    CatalogError::new(Reason::CatalogUnavailable, message)
}

/// Writes `Public/index.db` out of the source MSIX once its content is the
/// signed content, its identity is the WinGet source's, and it was published
/// no earlier than `index_not_before` (Unix seconds). The signature itself
/// is [`source::verify_signature`], over the same bytes.
pub fn extract_index(msix: &[u8], to: &Path, index_not_before: i64) -> Result<SourceIdentity> {
    let mut content = SourceContent::open(msix)?;
    let identity = content.identity()?;
    source::check_fresh(&identity, index_not_before)?;
    let bytes = content.entry(INDEX_ENTRY, INDEX_MAX_LEN)?;
    std::fs::write(to, &bytes)
        .map_err(|err| unavailable(format!("cannot write {}: {err}", to.display())))?;
    Ok(identity)
}

/// Looks an identifier up in an extracted index (case-insensitively, as
/// WinGet does). A row whose hash is not a SHA-256 is a
/// `catalog_integrity_failure`: its version data could not be verified.
pub fn lookup(index: &Path, package_identifier: &str) -> Result<Option<IndexRow>> {
    let conn = Connection::open_with_flags(
        index,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|err| unavailable(format!("cannot open the index: {err}")))?;
    let row = conn
        .query_row(
            "SELECT id, latest_version, hash FROM packages WHERE id = ?1 COLLATE NOCASE",
            [package_identifier],
            |row| {
                let hash = tigersetup_format::hex(&row.get::<_, Vec<u8>>(2)?);
                Ok(IndexRow {
                    id: row.get(0)?,
                    latest_version: row.get(1)?,
                    hash_prefix: hash.chars().take(8).collect(),
                    hash,
                })
            },
        )
        .optional()
        .map_err(|err| unavailable(format!("cannot query the index: {err}")))?;
    match row {
        Some(row) if row.hash.len() != 64 => Err(CatalogError::new(
            Reason::CatalogIntegrityFailure,
            format!(
                "the index lists {} with a {}-byte hash, not a SHA-256",
                row.id,
                row.hash.len() / 2
            ),
        )),
        row => Ok(row),
    }
}

/// Checks the downloaded (still compressed) version data against the
/// `packages.hash` the authenticated index lists for it.
pub fn verify_version_data(blob: &[u8], expected_sha256: &str) -> Result<()> {
    let actual = tigersetup_format::hex(&tigersetup_format::sha256(blob));
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(CatalogError::new(
            Reason::CatalogIntegrityFailure,
            format!("version data sha256 {actual} is not the index's {expected_sha256}"),
        ));
    }
    Ok(())
}

/// The version data URL for a package.
pub fn version_data_url(package_identifier: &str, hash_prefix: &str) -> String {
    format!("{CATALOG_BASE}packages/{package_identifier}/{hash_prefix}/versionData.mszyml")
}

/// Parses the inflated version data document.
pub fn parse_version_data(text: &str) -> Result<Vec<VersionEntry>> {
    let documents = YamlLoader::load_from_str(text)
        .map_err(|err| unavailable(format!("version data is not YAML: {err}")))?;
    let root = documents
        .first()
        .ok_or_else(|| unavailable("version data is empty"))?;
    let entries = root["vD"]
        .as_vec()
        .ok_or_else(|| unavailable("version data has no vD list"))?;
    let string = |node: &yaml_rust2::Yaml| -> Option<String> {
        match node {
            yaml_rust2::Yaml::String(s) => Some(s.clone()),
            yaml_rust2::Yaml::Integer(i) => Some(i.to_string()),
            yaml_rust2::Yaml::Real(r) => Some(r.clone()),
            _ => None,
        }
    };
    Ok(entries
        .iter()
        .filter_map(|entry| {
            Some(VersionEntry {
                version: string(&entry["v"])?,
                relative_path: string(&entry["rP"])?,
                sha256: string(&entry["s256H"])?.to_ascii_lowercase(),
            })
        })
        .collect())
}

/// The versions satisfying the requirement, highest first.
pub fn candidates(entries: &[VersionEntry], minimum_version: &str) -> Vec<VersionEntry> {
    let mut out: Vec<VersionEntry> = entries
        .iter()
        .filter(|e| version_satisfies(&e.version, minimum_version))
        .cloned()
        .collect();
    out.sort_by(|a, b| compare_versions(&b.version, &a.version));
    out
}

/// Fetches and verifies one merged manifest.
fn manifest_for(entry: &VersionEntry, cancel: Option<&AtomicBool>) -> Result<MergedManifest> {
    let url = format!("{CATALOG_BASE}{}", entry.relative_path);
    let bytes = catalog_get(&url, DOCUMENT_MAX_LEN, cancel)?;
    let actual = tigersetup_format::hex(&tigersetup_format::sha256(&bytes));
    if actual != entry.sha256 {
        return Err(unavailable(format!(
            "{url}: manifest sha256 {actual} is not the listed {}",
            entry.sha256
        )));
    }
    let text = String::from_utf8(bytes)
        .map_err(|err| unavailable(format!("{url}: manifest is not UTF-8: {err}")))?;
    manifest::parse(&text).map_err(|err| unavailable(format!("{url}: {err}")))
}

/// Resolves the current package version satisfying `requirement` and its
/// installer entry, accepting a source index published within the default
/// bound ([`ResolveOptions::default`]: the last 30 days by this machine's
/// clock). `work_dir` holds the extracted index for the duration of the
/// call.
pub fn resolve(
    package_identifier: &str,
    requirement: &Requirement<'_>,
    work_dir: &Path,
    cancel: Option<&AtomicBool>,
) -> Result<Resolved> {
    resolve_with(
        package_identifier,
        requirement,
        work_dir,
        cancel,
        &ResolveOptions::default(),
    )
}

/// [`resolve`] with an explicit judgement of the source index.
pub fn resolve_with(
    package_identifier: &str,
    requirement: &Requirement<'_>,
    work_dir: &Path,
    cancel: Option<&AtomicBool>,
    options: &ResolveOptions,
) -> Result<Resolved> {
    std::fs::create_dir_all(work_dir)
        .map_err(|err| unavailable(format!("cannot create {}: {err}", work_dir.display())))?;
    let index_path: PathBuf = work_dir.join(format!("index-{}.db", std::process::id()));
    let msix = catalog_get(INDEX_URL, INDEX_MAX_LEN, cancel)?;
    source::verify_signature(&msix)?;
    let found = extract_index(&msix, &index_path, options.index_not_before)
        .and_then(|identity| lookup(&index_path, package_identifier).map(|row| (identity, row)));
    let _ = std::fs::remove_file(&index_path);
    drop(msix);
    let (identity, row) = found?;
    let row = row.ok_or_else(|| {
        CatalogError::new(
            Reason::NoCompatibleVersion,
            format!("{package_identifier} is not in the catalog"),
        )
    })?;

    let url = version_data_url(&row.id, &row.hash_prefix);
    let blob = catalog_get(&url, DOCUMENT_MAX_LEN, cancel)?;
    verify_version_data(&blob, &row.hash).map_err(|err| CatalogError {
        message: format!("{url}: {}", err.message),
        ..err
    })?;
    let inflated = mszip::decompress(&blob, VERSION_DATA_MAX_LEN)
        .map_err(|err| unavailable(format!("{url}: {err}")))?;
    let text = String::from_utf8(inflated)
        .map_err(|err| unavailable(format!("{url}: version data is not UTF-8: {err}")))?;
    let entries = parse_version_data(&text)?;
    let candidates = candidates(&entries, requirement.minimum_version);
    if candidates.is_empty() {
        return Err(CatalogError::new(
            Reason::NoCompatibleVersion,
            format!(
                "{}: none of {} versions satisfies minimum {:?}",
                row.id,
                entries.len(),
                requirement.minimum_version
            ),
        ));
    }

    let mut last_error = None;
    for entry in candidates.iter().take(MANIFEST_ATTEMPTS) {
        let manifest = match manifest_for(entry, cancel) {
            Ok(manifest) => manifest,
            Err(err) if err.reason == Reason::CatalogUnavailable => {
                last_error = Some(err);
                continue;
            }
            Err(err) => return Err(err),
        };
        if let Some(installer) = manifest.select(requirement.architecture, requirement.scope) {
            return Ok(Resolved {
                index_version: identity.version,
                version: entry.version.clone(),
                manifest_path: entry.relative_path.clone(),
                installer,
            });
        }
    }
    Err(last_error.unwrap_or_else(|| {
        CatalogError::new(
            Reason::NoCompatibleVersion,
            format!(
                "{}: no {} installer in the {} newest satisfying versions",
                row.id,
                requirement.architecture,
                candidates.len().min(MANIFEST_ATTEMPTS)
            ),
        )
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::test_package::{manifest, package};

    /// The `packages` table exactly as the served index declares it.
    const PACKAGES_DDL: &str = "CREATE TABLE [packages](rowid INTEGER PRIMARY KEY, [id] TEXT NOT NULL, [name] TEXT NOT NULL, [moniker] TEXT, [latest_version] TEXT NOT NULL, [arp_min_version] TEXT, [arp_max_version] TEXT, [hash] BLOB)";

    fn hash_bytes() -> Vec<u8> {
        (0..32u8)
            .map(|i| [0x1f, 0x7c, 0xb3, 0xa6][i as usize % 4] ^ (i / 4))
            .collect()
    }

    #[test]
    fn index_inside_the_msix_is_extracted_and_queried() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("built.db");
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute_batch(PACKAGES_DDL).unwrap();
            conn.execute(
                "INSERT INTO packages (id, name, moniker, latest_version, hash) VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![
                    "Microsoft.EdgeWebView2Runtime",
                    "Microsoft Edge WebView2 Runtime",
                    "webview2",
                    "152.0.4191.53",
                    hash_bytes()
                ],
            )
            .unwrap();
        }
        let db = std::fs::read(&db_path).unwrap();
        let identity_manifest = manifest(
            source::SOURCE_NAME,
            source::SOURCE_PUBLISHER,
            "2026.1004.1308.18",
        );
        let msix = package(&identity_manifest, &db);
        let extracted = dir.path().join("index.db");
        let published = source::published_at("2026.1004.1308.18").unwrap();
        let identity = extract_index(&msix, &extracted, published).unwrap();
        assert_eq!(identity.version, "2026.1004.1308.18");
        assert_eq!(std::fs::read(&extracted).unwrap(), db);
        let row = lookup(&extracted, "microsoft.edgewebview2runtime")
            .unwrap()
            .unwrap();
        assert_eq!(row.id, "Microsoft.EdgeWebView2Runtime");
        assert_eq!(row.latest_version, "152.0.4191.53");
        assert_eq!(row.hash, tigersetup_format::hex(&hash_bytes()));
        assert_eq!(row.hash_prefix, "1f7cb3a6");
        assert_eq!(
            version_data_url(&row.id, &row.hash_prefix),
            "https://cdn.winget.microsoft.com/cache/packages/Microsoft.EdgeWebView2Runtime/1f7cb3a6/versionData.mszyml"
        );
        assert!(lookup(&extracted, "Vendor.Missing").unwrap().is_none());

        // Published a minute before the bound: refused before anything is
        // written.
        let refused = dir.path().join("refused.db");
        let err = extract_index(&msix, &refused, published + 60).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIndexStale);
        assert!(!refused.exists());
        // Not the WinGet source.
        let other = package(
            &manifest(
                "Contoso.Source",
                source::SOURCE_PUBLISHER,
                "2026.1004.1308.18",
            ),
            &db,
        );
        assert_eq!(
            extract_index(&other, &refused, 0).unwrap_err().reason,
            Reason::CatalogIdentityMismatch
        );
        assert_eq!(
            extract_index(b"not a zip", &extracted, 0)
                .unwrap_err()
                .reason,
            Reason::CatalogIntegrityFailure
        );

        // A row whose hash is not a SHA-256 cannot have its version data
        // verified.
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "INSERT INTO packages (id, name, latest_version, hash) VALUES ('Vendor.Short', 'Short', '1.0', X'1f7cb3a6')",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            lookup(&db_path, "Vendor.Short").unwrap_err().reason,
            Reason::CatalogIntegrityFailure
        );
    }

    #[test]
    fn version_data_must_hash_to_the_index_entry() {
        let blob =
            include_bytes!("../tests/fixtures/Microsoft.EdgeWebView2Runtime.versionData.mszyml");
        let listed = tigersetup_format::hex(&tigersetup_format::sha256(blob));
        verify_version_data(blob, &listed).unwrap();
        verify_version_data(blob, &listed.to_ascii_uppercase()).unwrap();
        let mut altered = blob.to_vec();
        let last = altered.len() - 1;
        altered[last] ^= 1;
        let err = verify_version_data(&altered, &listed).unwrap_err();
        assert_eq!(err.reason, Reason::CatalogIntegrityFailure);
        assert_eq!(err.reason.code(), "catalog_integrity_failure");
        assert_eq!(
            verify_version_data(blob, &listed[..8]).unwrap_err().reason,
            Reason::CatalogIntegrityFailure
        );
    }

    #[test]
    fn version_data_lists_every_version_and_candidates_are_ordered() {
        let blob =
            include_bytes!("../tests/fixtures/Microsoft.EdgeWebView2Runtime.versionData.mszyml");
        let text =
            String::from_utf8(mszip::decompress(blob, VERSION_DATA_MAX_LEN).unwrap()).unwrap();
        let entries = parse_version_data(&text).unwrap();
        assert_eq!(entries.len(), 141);
        assert_eq!(
            entries[0],
            VersionEntry {
                version: "152.0.4191.53".into(),
                relative_path: "manifests/m/Microsoft/EdgeWebView2Runtime/152.0.4191.53/f958"
                    .into(),
                sha256: "1c2a603527d660a1dd66852328e5f74e19b59f16ce929f66debdcc5fdb735122".into(),
            }
        );
        let any = candidates(&entries, "");
        assert_eq!(any.len(), 141);
        assert_eq!(any[0].version, "152.0.4191.53");
        let same_major = candidates(&entries, "151.0");
        assert!(same_major.iter().all(|e| e.version.starts_with("151.")));
        assert_eq!(same_major[0].version, "151.0.4129.107");
        assert!(candidates(&entries, "999.0").is_empty());
        // Order is by numeric comparison, not text.
        let mixed = vec![
            VersionEntry {
                version: "1.9.0".into(),
                relative_path: "a".into(),
                sha256: "x".into(),
            },
            VersionEntry {
                version: "1.10.0".into(),
                relative_path: "b".into(),
                sha256: "y".into(),
            },
        ];
        assert_eq!(candidates(&mixed, "1.0")[0].version, "1.10.0");
        assert_eq!(
            parse_version_data("sV: 1.0\n").unwrap_err().reason,
            Reason::CatalogUnavailable
        );
    }
}
