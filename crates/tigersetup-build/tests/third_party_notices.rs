//! The third-party notices TigerSetup distributes: the document lists exactly
//! the crates the release binaries link, and the builder carries it.

use std::path::{Path, PathBuf};
use std::process::Command;

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the repository root")
}

/// A crate added, removed or updated in `Cargo.lock` changes what the
/// binaries redistribute; the notices must say so before anything ships.
#[test]
fn the_notices_list_exactly_the_crates_the_release_binaries_link() {
    let root = repository_root();
    let output = Command::new("pwsh")
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(root.join("eng").join("Update-ThirdPartyNotices.ps1"))
        .arg("-Check")
        .current_dir(&root)
        .output()
        .expect("pwsh runs the notices check");
    assert!(
        output.status.success(),
        "THIRD-PARTY-NOTICES.md is not current; run pwsh -File eng\\Update-ThirdPartyNotices.ps1\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Every component with a notice obligation has its entry — TigerSetup's own
/// licence among them, since every generated installer carries TigerSetup's
/// code — and the zstd bindings' own BSD notice travels with the library's.
#[test]
fn the_notices_carry_every_component_with_an_obligation() {
    let notices = tigersetup_format::THIRD_PARTY_NOTICES;
    for expected in [
        "## TigerSetup",
        "Copyright (c) 2026 IT Tiger",
        "## Microsoft C runtime",
        "## Fluent UI System Icons",
        "Copyright (c) 2020 Microsoft Corporation",
        "## Zstandard",
        "Copyright (c) Meta Platforms, Inc. and affiliates.",
        "Copyright (c) 2026, Alexandre Bury",
        "## Rust standard library",
        "## SQLite",
        "| `prost` | ",
        "### Apache-2.0",
        "### MIT",
        "### Zlib",
    ] {
        assert!(notices.contains(expected), "the notices lack {expected:?}");
    }
}

#[test]
fn the_builder_prints_the_notices_it_carries() {
    let output = Command::new(env!("CARGO_BIN_EXE_tiger-setup"))
        .arg("notices")
        .output()
        .expect("tiger-setup notices runs");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        tigersetup_format::THIRD_PARTY_NOTICES
    );
}
