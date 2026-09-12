//! Compiles the Win32 resources the engine executable carries: the
//! side-by-side manifest (common controls v6, per-monitor-v2 DPI, long paths,
//! `asInvoker`), TigerSetup's icon and the version resource that identifies
//! TigerSetup itself. The builder replaces the icon and the version resource
//! in the copy it composes into each generated installer; the manifest is
//! the engine's own and travels unchanged.

#[path = "../build-support/version_resource.rs"]
mod version_resource;

use std::path::PathBuf;

fn main() {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    version_resource::compile(
        &version_resource::Executable {
            file_description: "TigerSetup installer engine",
            original_filename: "tigersetup-setup.exe",
            internal_name: "tigersetup-setup",
        },
        Some(&crate_dir.join("app.manifest")),
    );
}
