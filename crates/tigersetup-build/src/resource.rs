//! The Windows identity of a generated installer: the version resource and
//! the icons of the engine copy composed into it.
//!
//! The engine executable identifies TigerSetup, and it must: the release
//! binary is one file for every package, checked by hash. What Explorer
//! shows for `TigerMarkView-0.8.2-Setup.exe`, though, is the product being
//! installed, so the builder rewrites the resources of the *copy* of the
//! engine it composes into each installer. TigerSetup's provenance stays in
//! the metadata's `Engine` message and never in `ProductName` or
//! `FileDescription`. The rewrite goes through the Win32 resource-update API
//! on a temporary file rather than a PE rewriter of TigerSetup's own, and is
//! deterministic: the same engine, identity and icons give the same bytes.
//!
//! Every generated installer honours one resource-id contract:
//!
//! | resource | meaning |
//! |---|---|
//! | `RT_GROUP_ICON 1` | the executable's icon — what Explorer, the taskbar and the title bar show |
//! | `RT_GROUP_ICON 2` | TigerSetup's brand mark, always TigerSetup's own icon |
//! | `RT_VERSION 1` | the product's identity |
//!
//! All three are written in US English (`0x0409`), and every `RT_VERSION`,
//! `RT_GROUP_ICON` and `RT_ICON` resource of every language the engine had
//! is removed first, so nothing of TigerSetup's identity survives beside the
//! product's. When the executable's icon is TigerSetup's own, both groups
//! share one set of images. The raw engine has only group 1 — the wizard's
//! brand loader falls back to it — and the engine's side-by-side manifest is
//! left exactly as compiled.

pub mod icon;
pub mod pe;
pub mod version;

use std::path::Path;

pub use icon::Image;
pub use version::{Identity, padded_version};

use crate::Result;

/// TigerSetup's own icon: the file the build scripts compile into both
/// TigerSetup executables, embedded so that there is one source for it.
pub const TIGERSETUP_ICON: &[u8] = include_bytes!("../../../docs/assets/TigerSetup.ico");

/// The ids of the contract above.
pub const EXECUTABLE_ICON_ID: u16 = 1;
pub const BRAND_ICON_ID: u16 = 2;
pub const VERSION_ID: u16 = 1;

/// The engine bytes with `identity` as their version resource, `exe_icon`
/// as `RT_GROUP_ICON 1` and `brand_icon` as `RT_GROUP_ICON 2`. Both icons
/// are the bytes of an `.ico` file. The work happens on a temporary copy in
/// the builder's temporary directory, removed on every path.
pub fn apply(
    engine: &[u8],
    identity: &Identity,
    exe_icon: &[u8],
    brand_icon: &[u8],
) -> Result<Vec<u8>> {
    let exe_images = icon::parse("the installer's icon", exe_icon)?;
    let brand_images = if brand_icon == exe_icon {
        None
    } else {
        Some(icon::parse("the brand icon", brand_icon)?)
    };
    let version_blob = version::encode(identity);

    let dir = tempfile::Builder::new().prefix("tiger-setup-").tempdir()?;
    let path = dir.path().join("engine.exe");
    std::fs::write(&path, engine)?;

    let existing = {
        let module = pe::Module::open(&path)?;
        module.existing(&[pe::RT_VERSION, pe::RT_GROUP_ICON, pe::RT_ICON])?
    };

    let mut writes = Vec::new();
    let mut next_image = 1u16;
    let mut add_images = |images: &[Image], writes: &mut Vec<pe::Update>| -> u16 {
        let first = next_image;
        for image in images {
            writes.push(pe::Update {
                kind: pe::RT_ICON,
                id: next_image,
                data: image.bytes.clone(),
            });
            next_image += 1;
        }
        first
    };
    let exe_first = add_images(&exe_images, &mut writes);
    writes.push(pe::Update {
        kind: pe::RT_GROUP_ICON,
        id: EXECUTABLE_ICON_ID,
        data: icon::group(&exe_images, exe_first),
    });
    let brand_group = match &brand_images {
        Some(images) => {
            let first = add_images(images, &mut writes);
            icon::group(images, first)
        }
        None => icon::group(&exe_images, exe_first),
    };
    writes.push(pe::Update {
        kind: pe::RT_GROUP_ICON,
        id: BRAND_ICON_ID,
        data: brand_group,
    });
    writes.push(pe::Update {
        kind: pe::RT_VERSION,
        id: VERSION_ID,
        data: version_blob,
    });

    pe::update(&path, &existing, &writes, version::LANGUAGE)?;
    Ok(std::fs::read(&path)?)
}

/// The images of icon group `id` of the executable at `path`, in the
/// group's order, or `None` when the file has no such group. This is what
/// `inspect` reports and what the tests compare with the `.ico` that went in.
pub fn read_group_icon(path: &Path, id: u16) -> Result<Option<Vec<Image>>> {
    let module = pe::Module::open(path)?;
    let Some(directory) = module.resource(pe::RT_GROUP_ICON, id)? else {
        return Ok(None);
    };
    let mut images = Vec::new();
    for (directory, image_id) in icon::group_entries(&directory)? {
        let bytes = module.resource(pe::RT_ICON, image_id)?.ok_or_else(|| {
            crate::BuildError::new(
                "resource_unreadable",
                format!(
                    "{}: icon group {id} names image {image_id}, which the file does not have",
                    path.display()
                ),
            )
        })?;
        images.push(Image { directory, bytes });
    }
    Ok(Some(images))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::version_info;
    use std::path::PathBuf;

    /// A real PE with a version resource and icons of its own, which the
    /// rewrite must replace rather than add to.
    pub fn inbox_executable() -> PathBuf {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        PathBuf::from(root).join("System32").join("notepad.exe")
    }

    fn sample_identity() -> Identity {
        Identity {
            company_name: "IT Tiger".into(),
            product_name: "TigerMarkView".into(),
            product_version: "0.8.2".into(),
            file_version: padded_version("0.8.2"),
            file_description: "TigerMarkView Setup".into(),
            legal_copyright: "Copyright (c) 2026 IT Tiger".into(),
            original_filename: "TigerMarkView-0.8.2-Setup.exe".into(),
            internal_name: "TigerMarkView-0.8.2-Setup".into(),
        }
    }

    /// A two-image icon with recognisable bytes, unlike TigerSetup's.
    fn other_icon() -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, 2, 0];
        let first = 6 + 2 * 16;
        let images: [&[u8]; 2] = [b"first image bytes", b"second image, longer bytes"];
        let mut offset = first;
        for (width, image) in [16u8, 32].iter().zip(images) {
            bytes.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            bytes.extend_from_slice(&(image.len() as u32).to_le_bytes());
            bytes.extend_from_slice(&(offset as u32).to_le_bytes());
            offset += image.len();
        }
        for image in images {
            bytes.extend_from_slice(image);
        }
        bytes
    }

    fn images_of(ico: &[u8]) -> Vec<Image> {
        icon::parse("test", ico).unwrap()
    }

    #[test]
    fn the_identity_replaces_the_version_resource_and_reads_back_whole() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let identity = sample_identity();
        let rewritten = apply(&engine, &identity, TIGERSETUP_ICON, TIGERSETUP_ICON).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Sample-Setup.exe");
        std::fs::write(&path, &rewritten).unwrap();

        let info = version_info::read(&path).unwrap();
        assert_eq!(info.company_name, "IT Tiger");
        assert_eq!(info.product_name, "TigerMarkView");
        assert_eq!(info.product_version, "0.8.2");
        assert_eq!(info.file_version, "0.8.2.0");
        assert_eq!(info.file_version_string, "0.8.2.0");
        assert_eq!(info.file_description, "TigerMarkView Setup");
        assert_eq!(info.legal_copyright, "Copyright (c) 2026 IT Tiger");
        assert_eq!(info.original_filename, "TigerMarkView-0.8.2-Setup.exe");
        assert_eq!(info.internal_name, "TigerMarkView-0.8.2-Setup");
        assert_eq!(info.translation, "040904b0");
        assert!(info.comments.is_empty(), "nothing of the original survives");

        // Nothing of the original version resource remains in any language.
        let module = pe::Module::open(&path).unwrap();
        let versions = module.existing(&[pe::RT_VERSION]).unwrap();
        assert_eq!(
            versions,
            vec![pe::Existing {
                kind: pe::RT_VERSION,
                name: pe::Name::Id(1),
                language: 0x0409
            }]
        );
    }

    #[test]
    fn an_empty_copyright_is_omitted_rather_than_written_empty() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let identity = Identity {
            legal_copyright: String::new(),
            ..sample_identity()
        };
        let rewritten = apply(&engine, &identity, TIGERSETUP_ICON, TIGERSETUP_ICON).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Sample-Setup.exe");
        std::fs::write(&path, &rewritten).unwrap();
        let info = version_info::read(&path).unwrap();
        assert_eq!(info.product_name, "TigerMarkView");
        assert_eq!(info.legal_copyright, "");
    }

    #[test]
    fn the_icon_groups_reproduce_the_ico_files_exactly() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let other = other_icon();
        let rewritten = apply(&engine, &sample_identity(), &other, TIGERSETUP_ICON).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Sample-Setup.exe");
        std::fs::write(&path, &rewritten).unwrap();

        let executable = read_group_icon(&path, EXECUTABLE_ICON_ID).unwrap().unwrap();
        assert_eq!(executable, images_of(&other));
        assert_eq!(executable.len(), 2);
        assert_eq!(executable[1].bytes, b"second image, longer bytes");

        // TigerSetup's own icon is the small 8-bit artwork: six sizes from
        // 64 down to 16 pixels, no 256-pixel image, nothing PNG-encoded —
        // what a loader and an installer's title bar need and no more.
        let brand = read_group_icon(&path, BRAND_ICON_ID).unwrap().unwrap();
        assert_eq!(brand, images_of(TIGERSETUP_ICON));
        assert_eq!(brand.len(), 6);
        assert_eq!(
            brand.iter().map(|image| image.width()).collect::<Vec<_>>(),
            vec![64, 48, 32, 24, 20, 16]
        );
        assert!(brand.iter().all(|image| image.bits() == 8));
        assert!(
            brand
                .iter()
                .all(|image| !image.bytes.starts_with(b"\x89PNG")),
            "no entry is PNG-encoded"
        );

        // Two distinct icons: 2 + 6 images, ids 1..=8, and no others.
        let module = pe::Module::open(&path).unwrap();
        let images = module.existing(&[pe::RT_ICON]).unwrap();
        let ids: Vec<u16> = images
            .iter()
            .map(|e| match &e.name {
                pe::Name::Id(id) => *id,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(ids, (1..=8).collect::<Vec<u16>>());
        assert!(images.iter().all(|e| e.language == 0x0409));
        assert!(read_group_icon(&path, 3).unwrap().is_none());
    }

    #[test]
    fn identical_icons_share_one_set_of_images() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let rewritten = apply(
            &engine,
            &sample_identity(),
            TIGERSETUP_ICON,
            TIGERSETUP_ICON,
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Sample-Setup.exe");
        std::fs::write(&path, &rewritten).unwrap();
        let module = pe::Module::open(&path).unwrap();
        assert_eq!(module.existing(&[pe::RT_ICON]).unwrap().len(), 6);
        assert_eq!(module.existing(&[pe::RT_GROUP_ICON]).unwrap().len(), 2);
        drop(module);
        let executable = read_group_icon(&path, EXECUTABLE_ICON_ID).unwrap().unwrap();
        let brand = read_group_icon(&path, BRAND_ICON_ID).unwrap().unwrap();
        assert_eq!(executable, brand);
        assert_eq!(executable, images_of(TIGERSETUP_ICON));
    }

    #[test]
    fn the_same_inputs_give_the_same_bytes() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let identity = sample_identity();
        let first = apply(&engine, &identity, &other_icon(), TIGERSETUP_ICON).unwrap();
        let second = apply(&engine, &identity, &other_icon(), TIGERSETUP_ICON).unwrap();
        assert_eq!(first, second);
        assert_ne!(first, engine);
        // Applying to the rewritten bytes again replaces, and lands on the
        // same result: the rewrite is idempotent.
        let third = apply(&first, &identity, &other_icon(), TIGERSETUP_ICON).unwrap();
        assert_eq!(first, third);
    }

    #[test]
    fn an_unusable_icon_or_engine_is_refused_with_a_stable_code() {
        let engine = std::fs::read(inbox_executable()).unwrap();
        let err = apply(&engine, &sample_identity(), b"not an icon", TIGERSETUP_ICON).unwrap_err();
        assert_eq!(err.code, "icon_invalid");
        let err = apply(&engine, &sample_identity(), TIGERSETUP_ICON, b"").unwrap_err();
        assert_eq!(err.code, "icon_invalid");
        let err = apply(
            b"not a portable executable",
            &sample_identity(),
            TIGERSETUP_ICON,
            TIGERSETUP_ICON,
        )
        .unwrap_err();
        assert_eq!(err.code, "engine_invalid");
    }
}
