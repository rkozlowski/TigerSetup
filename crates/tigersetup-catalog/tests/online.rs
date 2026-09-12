//! Resolves two real packages against the live WinGet source. Runs only
//! with `TIGERSETUP_ONLINE_TESTS=1`; otherwise it reports itself skipped and
//! passes, so the offline suite never depends on the network.

use std::time::Instant;

use tigersetup_catalog::winget::{Requirement, resolve};
use tigersetup_format::identity::Scope;

#[test]
fn resolves_real_packages_from_the_live_catalog() {
    if std::env::var("TIGERSETUP_ONLINE_TESTS").as_deref() != Ok("1") {
        eprintln!("SKIPPED: set TIGERSETUP_ONLINE_TESTS=1 to resolve against the live catalog");
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
            "ONLINE {id}: version {} type {} scope {:?} elevation {} url {} sha256 {} args {:?} success {:?} reboot {:?} manifest {} in {} ms",
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
