//! Compiles the Win32 resources the two executables of this package carry:
//! the side-by-side manifest (common controls v6, per-monitor-v2 DPI, long
//! paths, `asInvoker`), TigerSetup's icon and the version resource that
//! identifies TigerSetup itself. The builder replaces the icon and the
//! version resource in the copies it composes into each generated
//! installer; the manifest is each executable's own and travels unchanged.

#[path = "../build-support/version_resource.rs"]
mod version_resource;

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let manifest = crate_dir.join("app.manifest");
    version_resource::compile(
        &version_resource::Executable {
            binary: "tigersetup-setup",
            file_description: "TigerSetup installer engine",
            original_filename: "tigersetup-setup.exe",
            internal_name: "tigersetup-setup",
        },
        Some(&manifest),
    );
    version_resource::compile(
        &version_resource::Executable {
            binary: "tigersetup-loader",
            file_description: "TigerSetup installer",
            original_filename: "tigersetup-loader.exe",
            internal_name: "tigersetup-loader",
        },
        Some(&manifest),
    );
}
