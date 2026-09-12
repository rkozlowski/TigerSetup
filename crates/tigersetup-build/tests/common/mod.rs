//! Shared support for the builder's process-level tests: a real installer
//! built from a manifest, a payload and a stand-in engine.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use tigersetup_build::{BuildRequest, build};

/// A manifest, a payload and an engine in a directory of their own, built
/// into an installer.
pub fn built_installer(root: &Path, manifest_text: &str) -> PathBuf {
    std::fs::create_dir_all(root.join("payload/bin")).unwrap();
    std::fs::write(root.join("payload/bin/app.exe"), b"application bytes").unwrap();
    std::fs::write(root.join("payload/readme.txt"), b"read me").unwrap();
    std::fs::write(root.join("TigerSetup.toml"), manifest_text).unwrap();
    // A real PE stands in for the engine: the builder gives its copy the
    // product's identity through the Win32 resource API, which needs one.
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let engine = root.join("engine.exe");
    std::fs::copy(
        PathBuf::from(system_root)
            .join("System32")
            .join("notepad.exe"),
        &engine,
    )
    .unwrap();
    let output = root.join("out");
    let result = build(&BuildRequest {
        manifest_path: &root.join("TigerSetup.toml"),
        output: &output,
        engine_path: Some(&engine),
        properties: &[],
        offline: true,
        // Tests build many installers; the payload's size is not what they
        // measure.
        compression: tigersetup_build::Compression::Fast,
    })
    .unwrap();
    result.installer_path
}
