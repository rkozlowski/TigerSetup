//! Authenticates the live source package and resolves two real packages
//! against the live WinGet source. Runs only with
//! `TIGERSETUP_ONLINE_TESTS=1`; otherwise it reports itself skipped and
//! passes, so the offline suite never depends on the network.

use std::time::Instant;

use tigersetup_catalog::Reason;
use tigersetup_catalog::source::{self, SourceContent};
use tigersetup_catalog::winget::{INDEX_ENTRY, INDEX_URL, Requirement, extract_index, resolve};
use tigersetup_format::identity::Scope;

fn online() -> bool {
    if std::env::var("TIGERSETUP_ONLINE_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIPPED: set TIGERSETUP_ONLINE_TESTS=1 to run against the live catalog");
        return false;
    }
    true
}

#[test]
fn the_live_source_package_authenticates_and_a_tampered_copy_does_not() {
    if !online() {
        return;
    }
    let msix = tigersetup_catalog::http::fetch(INDEX_URL, 256 << 20, None).unwrap();
    let started = Instant::now();
    source::verify_signature(&msix).unwrap_or_else(|err| panic!("{err}"));
    let signature_ms = started.elapsed().as_millis();

    let mut content = SourceContent::open(&msix).unwrap();
    let identity = content.identity().unwrap();
    let index = content.entry(INDEX_ENTRY, 256 << 20).unwrap();
    eprintln!(
        "ONLINE source {}: published {:?} (Unix seconds), {} bytes, index {} bytes, signature {signature_ms} ms",
        identity.version,
        identity.published_at,
        msix.len(),
        index.len()
    );

    let dir = tempfile::tempdir().unwrap();
    let stale = extract_index(
        &msix,
        &dir.path().join("index.db"),
        identity
            .published_at
            .expect("the live index names its publication time")
            + 1,
    )
    .unwrap_err();
    assert_eq!(stale.reason, Reason::CatalogIndexStale);

    let mut tampered = msix.clone();
    let middle = tampered.len() / 2;
    tampered[middle] ^= 0xff;
    let err = source::verify_signature(&tampered).unwrap_err();
    assert_eq!(err.reason, Reason::CatalogSignatureInvalid, "{err}");
    eprintln!("ONLINE tampered source: {err}");
}

#[test]
fn resolves_real_packages_from_the_live_catalog() {
    if !online() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    for (id, minimum, scope) in [
        ("Microsoft.DotNet.DesktopRuntime.10", "10.0", Scope::User),
        ("Microsoft.EdgeWebView2Runtime", "", Scope::User),
    ] {
        let started = Instant::now();
        let requirement = Requirement {
            minimum_version: minimum,
            architecture: "x64",
            scope,
        };
        let resolved =
            resolve(id, &requirement, dir.path(), None).unwrap_or_else(|err| panic!("{id}: {err}"));
        eprintln!(
            "ONLINE {id}: index {} version {} type {} scope {:?} elevation {} url {} sha256 {} args {:?} success {:?} reboot {:?} manifest {} in {} ms",
            resolved.index_version,
            resolved.version,
            resolved.installer.installer_type,
            resolved.installer.scope,
            resolved.installer.elevation_required,
            resolved.installer.url,
            resolved.installer.sha256,
            resolved.installer.arguments,
            resolved.installer.success_codes,
            resolved.installer.reboot_codes,
            resolved.manifest_path,
            started.elapsed().as_millis()
        );
        assert!(resolved.installer.url.starts_with("https://"));
        assert_eq!(resolved.installer.sha256.len(), 64);
        assert_eq!(resolved.installer.architecture, "x64");
        assert!(!resolved.installer.arguments.is_empty());
        if !minimum.is_empty() {
            assert!(resolved.version.starts_with("10."));
        }
    }
}
