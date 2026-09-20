//! Compiles the Win32 resources the engine carries: the side-by-side
//! manifest (common controls v6, per-monitor-v2 DPI, long paths,
//! `asInvoker`), TigerSetup's icon and the version resource that identifies
//! TigerSetup itself. The builder replaces the icon and the version
//! resource in the copy it composes into each generated installer; the
//! manifest is the executable's own and travels unchanged. The script also
//! hands the process-level tests the loader `tigersetup-loader` built.

#[path = "../build-support/version_resource.rs"]
mod version_resource;

fn main() {
    let manifest = version_resource::repository_root().join("crates/build-support/app.manifest");
    version_resource::compile(
        &version_resource::Executable {
            binary: "tigersetup-setup",
            file_description: "TigerSetup installer engine",
            original_filename: "tigersetup-setup.exe",
            internal_name: "tigersetup-setup",
            icon: version_resource::SETUP_ICON,
        },
        Some(&manifest),
    );
    // The loader the tests compose installers with, where `tigersetup-loader`'s
    // build script put it.
    let loader = std::env::var("DEP_TIGERSETUP_LOADER_LOADER")
        .expect("tigersetup-loader's build script announces the loader");
    println!("cargo:rustc-env=TIGERSETUP_LOADER_EXE={loader}");
}
