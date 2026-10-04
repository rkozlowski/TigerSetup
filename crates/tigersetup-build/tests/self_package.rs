//! TigerSetup's own package (`packages/tigersetup/`) as a WinGet submission
//! reads it: first-party URLs for the repository, its issues, the release's
//! notes, and the licence and privacy statement frozen with the release —
//! each naming a document the repository really has — and a
//! description that says what the installer does to the machine. The
//! package installs its licence and notices beside the builder.

use std::path::{Path, PathBuf};

use tigersetup_build::manifest::LoadedManifest;
use tigersetup_build::winget::VERSION_PLACEHOLDER;

const REPOSITORY: &str = "https://github.com/rkozlowski/TigerSetup";

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

fn package() -> LoadedManifest {
    LoadedManifest::load(&repository_root().join("packages/tigersetup/TigerSetup.toml"))
        .expect("TigerSetup's own package loads")
}

#[test]
fn the_winget_urls_are_first_party_and_name_documents_the_repository_has() {
    let loaded = package();
    let winget = &loaded.manifest.winget;
    let root = repository_root();

    assert_eq!(winget.package_url.as_deref(), Some(REPOSITORY));
    assert_eq!(
        winget.publisher_support_url.as_deref(),
        Some(format!("{REPOSITORY}/issues").as_str())
    );
    assert_eq!(
        winget.release_notes_url.as_deref(),
        Some(format!("{REPOSITORY}/releases/tag/v{VERSION_PLACEHOLDER}").as_str()),
        "the notes of the release the manifests are for"
    );
    assert!(root.join("LICENSE.txt").is_file());
    assert!(root.join("PRIVACY.md").is_file());

    let privacy = std::fs::read_to_string(root.join("PRIVACY.md")).unwrap();
    for topic in [
        "IT Tiger",
        "no telemetry",
        "state database",
        "Restart Manager",
        "Copy log path",
        "cdn.winget.microsoft.com",
        "--offline",
        "--no-dependency-install",
        "dotnet msbuild",
        "A successful uninstall removes everything TigerSetup added",
        "releases/download/v<version>/PRIVACY.md",
    ] {
        assert!(
            privacy.contains(topic),
            "PRIVACY.md does not cover {topic:?}"
        );
    }
}

/// The licence and privacy statement a version's WinGet manifests name are
/// the ones frozen with that version: the `LICENSE.txt` and `PRIVACY.md`
/// assets of its GitHub Release (`RELEASING.md`), whose URL the release tag
/// fixes before the release exists — never a branch or a page that changes
/// when a later version does.
#[test]
fn the_licence_and_privacy_urls_name_the_terms_frozen_with_the_version() {
    let loaded = package();
    let winget = &loaded.manifest.winget;
    let frozen =
        |asset: &str| format!("{REPOSITORY}/releases/download/v{VERSION_PLACEHOLDER}/{asset}");

    let declared = [
        ("LicenseUrl", winget.license_url.as_deref(), "LICENSE.txt"),
        ("PrivacyUrl", winget.privacy_url.as_deref(), "PRIVACY.md"),
    ];
    for (field, url, asset) in declared {
        let url = url.unwrap_or_else(|| panic!("TigerSetup declares no {field}"));
        assert_eq!(
            url,
            frozen(asset),
            "{field} is not the release's frozen {asset}"
        );
        assert!(
            url.contains(VERSION_PLACEHOLDER),
            "{field} is not bound to the version"
        );
        for mutable in [
            "/main/", "/master/", "/blob/", "/raw/", "/latest/", "/HEAD/",
        ] {
            assert!(!url.contains(mutable), "{field} {url} names {mutable}");
        }

        // This build's version in the placeholder: the release-asset URL its
        // manifests carry (the generator's own substitution is proved in
        // winget_manifests.rs, and the release build checks the manifests).
        let version = env!("CARGO_PKG_VERSION");
        assert_eq!(
            url.replace(VERSION_PLACEHOLDER, version),
            format!("{REPOSITORY}/releases/download/v{version}/{asset}")
        );
    }
}

#[test]
fn the_description_says_what_the_installer_does_to_the_machine() {
    let loaded = package();
    let description = loaded.manifest.winget.description.clone().unwrap();
    for behaviour in [
        "for the current user by default",
        "PATH",
        "optional",
        "administrator approval",
        "WinGet community catalog",
        "--offline",
        ".NET SDK",
        "no telemetry",
        "Uninstalling removes everything",
    ] {
        assert!(
            description.contains(behaviour),
            "the WinGet description does not say {behaviour:?}"
        );
    }
}

#[test]
fn the_package_installs_its_licence_and_notices() {
    let script =
        std::fs::read_to_string(repository_root().join("packages/tigersetup/Build-Package.ps1"))
            .unwrap();
    for staged in ["'LICENSE.txt'", "'THIRD-PARTY-NOTICES.md'"] {
        assert!(
            script.contains(staged),
            "Build-Package.ps1 does not stage {staged}"
        );
    }
}
