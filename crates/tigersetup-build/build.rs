//! Compiles the Win32 resources the builder executable carries: TigerSetup's
//! icon and the version resource that identifies TigerSetup itself. The
//! builder is a console tool and needs no side-by-side manifest.

#[path = "../build-support/version_resource.rs"]
mod version_resource;

fn main() {
    version_resource::compile(
        &version_resource::Executable {
            file_description: "TigerSetup builder",
            original_filename: "tiger-setup.exe",
            internal_name: "tiger-setup",
        },
        None,
    );
}
