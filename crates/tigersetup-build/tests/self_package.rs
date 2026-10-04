//! TigerSetup's own package (`packages/tigersetup/`) as a WinGet submission
//! reads it: first-party URLs for the repository, its issues, the release's
//! notes and licence at the release tag, and the product's own privacy
//! statement — each naming a document the repository really has — and a
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
    assert_eq!(
        winget.license_url.as_deref(),
        Some(format!("{REPOSITORY}/blob/v{VERSION_PLACEHOLDER}/LICENSE.txt").as_str()),
        "the licence at the release tag"
    );
    assert!(root.join("LICENSE.txt").is_file());

    // The privacy statement is the product's own, at a stable address.
    assert_eq!(
        winget.privacy_url.as_deref(),
        Some(format!("{REPOSITORY}/blob/main/PRIVACY.md").as_str())
    );
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
        "blob/main/PRIVACY.md",
    ] {
        assert!(
            privacy.contains(topic),
            "PRIVACY.md does not cover {topic:?}"
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
