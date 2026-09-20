//! The Windows identity of a TigerSetup executable: a `VERSIONINFO` resource
//! naming TigerSetup as the product, and TigerSetup's icon as resource id 1.
//! Every TigerSetup executable — the engine, the loader and the builder —
//! compiles it from this one file, included by each build script with a
//! `#[path]` module declaration, so what they say about the product is
//! written down once.
//!
//! The resource script is generated rather than checked in for one reason:
//! the version belongs to `Cargo.toml` and must not be written down twice.
//! The icon the script refers to is the repository's own
//! `docs/assets/TigerSetup.ico`, named with an absolute path so that the
//! resource compiler's working directory cannot matter.
//!
//! `ProductVersion` is the crate version exactly (`Major.Minor.Patch`, which
//! is TigerSetup's public version format); `FileVersion` and the fixed block
//! carry the same version padded with `.0` to the four numeric parts Windows
//! requires.

use std::path::{Path, PathBuf};

/// The product the strings name. Fixed for every TigerSetup executable.
pub const COMPANY_NAME: &str = "IT Tiger";
pub const PRODUCT_NAME: &str = "TigerSetup";
pub const LEGAL_COPYRIGHT: &str = "Copyright (c) 2026 IT Tiger";

/// What differs between the executables.
pub struct Executable {
    /// The Cargo binary target the resource is linked into.
    pub binary: &'static str,
    pub file_description: &'static str,
    pub original_filename: &'static str,
    pub internal_name: &'static str,
}

/// Compiles the version resource and the icon into the crate's binary. A
/// crate whose executable also needs a side-by-side manifest names it in
/// `manifest`; it is then resource 1 of type `RT_MANIFEST` (24) and its
/// absence is an error, because a wizard without it draws unthemed controls.
pub fn compile(executable: &Executable, manifest: Option<&Path>) {
    let crate_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let repository = crate_dir
        .parent()
        .and_then(Path::parent)
        .expect("the crate lives two levels below the repository root")
        .to_path_buf();
    let shared = repository.join("crates/build-support/version_resource.rs");
    let icon = repository.join("docs/assets/TigerSetup.ico");
    assert!(icon.is_file(), "{} is missing", icon.display());

    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let padded = padded_version(&version);
    let numeric = padded
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let dotted = padded
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(".");

    let manifest_line = manifest
        .map(|path| format!("1 24 \"{}\"\n", rc_path(path)))
        .unwrap_or_default();
    let script = format!(
        r#"{manifest_line}1 ICON "{icon}"

1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEFLAGSMASK 0x3fL
FILEFLAGS 0x0L
FILEOS 0x40004L
FILETYPE 0x1L
FILESUBTYPE 0x0L
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904b0"
        BEGIN
            VALUE "CompanyName", "{COMPANY_NAME}"
            VALUE "FileDescription", "{file_description}"
            VALUE "FileVersion", "{dotted}"
            VALUE "InternalName", "{internal_name}"
            VALUE "LegalCopyright", "{LEGAL_COPYRIGHT}"
            VALUE "OriginalFilename", "{original_filename}"
            VALUE "ProductName", "{PRODUCT_NAME}"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = rc_path(&icon),
        file_description = executable.file_description,
        internal_name = executable.internal_name,
        original_filename = executable.original_filename,
    );

    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let script_path = out_dir.join(format!("{}.rc", executable.binary));
    std::fs::write(&script_path, script).expect("the resource script is written");

    let result =
        embed_resource::compile_for(&script_path, [executable.binary], embed_resource::NONE);
    match manifest {
        Some(_) => result
            .manifest_required()
            .expect("the resource compiler produced the manifest"),
        None => result
            .manifest_optional()
            .expect("the resource compiler produced the version resource"),
    }

    // Listing any file switches Cargo from "rerun on any change in the
    // package" to "rerun on these", so everything the script depends on is
    // named: the inputs, this file, the build script and the version.
    if let Some(path) = manifest {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed={}", shared.display());
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=CARGO_PKG_VERSION");
}

/// The numeric parts of a `Major.Minor.Patch` version padded with zeros to
/// the four parts a Windows version resource carries.
fn padded_version(version: &str) -> [u32; 4] {
    let mut parts = [0u32; 4];
    for (slot, part) in parts.iter_mut().zip(version.split('.')) {
        *slot = part
            .parse()
            .unwrap_or_else(|_| panic!("CARGO_PKG_VERSION {version:?} is not Major.Minor.Patch"));
    }
    parts
}

/// A path as a resource-script string literal: backslashes doubled.
fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "\\\\")
}
