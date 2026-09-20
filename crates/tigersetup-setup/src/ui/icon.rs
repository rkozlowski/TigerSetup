//! Icons the wizard draws: the product's branding icon, carried in the
//! metadata as the raw bytes of an `.ico` file, and the two icon resources
//! every generated installer carries — the executable's own (group 1, what
//! Explorer shows for the file, which the window's title bar and taskbar
//! entry repeat) and TigerSetup's brand mark (group 2). The builder writes
//! both into the engine copy it composes; the raw engine has only group 1.
//!
//! An `.ico` file is a directory of images at several sizes. Windows can
//! build an icon from one of those images (`CreateIconFromResourceEx`), so
//! the directory is parsed here to choose the entry closest to the size the
//! wizard is about to draw at — which changes with the dpi.

use windows_sys::Win32::Foundation::HINSTANCE;
use windows_sys::Win32::System::LibraryLoader::{
    FindResourceW, LoadResource, LockResource, SizeofResource,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateIconFromResourceEx, HICON, IMAGE_ICON, LR_DEFAULTCOLOR, LoadImageW, RT_GROUP_ICON,
};

/// The executable's icon: what Explorer shows for the installer file.
pub const EXECUTABLE_ICON_ID: u16 = 1;
/// TigerSetup's brand mark, always TigerSetup's own icon.
pub const BRAND_ICON_ID: u16 = 2;

/// The version word `CreateIconFromResourceEx` wants for an icon or cursor
/// image taken from a resource: 3.0.
const ICON_RESOURCE_VERSION: u32 = 0x0003_0000;

const HEADER_LEN: usize = 6;
const ENTRY_LEN: usize = 16;
const ICO_TYPE: u16 = 1;

/// One image in an `.ico` directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    /// Pixels; the file stores 0 for 256.
    width: u32,
    offset: usize,
    length: usize,
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The images an `.ico` file declares, ignoring any whose bytes are not
/// inside the file. An empty result means the bytes are not a usable icon.
fn directory(bytes: &[u8]) -> Vec<Entry> {
    if bytes.len() < HEADER_LEN || u16_at(bytes, 0) != 0 || u16_at(bytes, 2) != ICO_TYPE {
        return Vec::new();
    }
    let count = u16_at(bytes, 4) as usize;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = HEADER_LEN + index * ENTRY_LEN;
        if at + ENTRY_LEN > bytes.len() {
            break;
        }
        let width = match bytes[at] {
            0 => 256,
            other => other as u32,
        };
        let length = u32_at(bytes, at + 8) as usize;
        let offset = u32_at(bytes, at + 12) as usize;
        if length == 0 || offset.saturating_add(length) > bytes.len() {
            continue;
        }
        entries.push(Entry {
            width,
            offset,
            length,
        });
    }
    entries
}

/// The image to draw at `wanted` pixels: the exact size when the file has
/// it, otherwise the smallest larger one (downscaling beats upscaling), and
/// otherwise the largest available.
fn best(entries: &[Entry], wanted: u32) -> Option<Entry> {
    entries
        .iter()
        .filter(|entry| entry.width >= wanted)
        .min_by_key(|entry| entry.width)
        .or_else(|| entries.iter().max_by_key(|entry| entry.width))
        .copied()
}

/// Builds an icon at `size` pixels from the bytes of an `.ico` file.
pub fn from_ico(bytes: &[u8], size: i32) -> Option<HICON> {
    let entry = best(&directory(bytes), size.max(1) as u32)?;
    let image = &bytes[entry.offset..entry.offset + entry.length];
    let icon = unsafe {
        CreateIconFromResourceEx(
            image.as_ptr(),
            image.len() as u32,
            1,
            ICON_RESOURCE_VERSION,
            size,
            size,
            LR_DEFAULTCOLOR,
        )
    };
    (!icon.is_null()).then_some(icon)
}

fn resource(instance: HINSTANCE, id: u16, size: i32) -> HICON {
    unsafe {
        LoadImageW(
            instance,
            id as usize as *const u16,
            IMAGE_ICON,
            size,
            size,
            LR_DEFAULTCOLOR,
        ) as HICON
    }
}

/// The executable's own icon at `size` pixels.
pub fn executable(instance: HINSTANCE, size: i32) -> HICON {
    resource(instance, EXECUTABLE_ICON_ID, size)
}

/// The icon to show for the product: its branding icon where the package
/// carries one and it can be read, the executable's otherwise.
pub fn product(icon_bytes: &[u8], instance: HINSTANCE, size: i32) -> HICON {
    from_ico(icon_bytes, size).unwrap_or_else(|| executable(instance, size))
}

/// A `GRPICONDIRENTRY` of an icon group resource: 14 bytes, of which the
/// first is the image's width (0 for 256).
const GROUP_HEADER_LEN: usize = 6;
const GROUP_ENTRY_LEN: usize = 14;

/// The widths of the images an icon group resource of this executable
/// holds, or nothing when the group is not there.
fn group_widths(instance: HINSTANCE, id: u16) -> Vec<u32> {
    let bytes: &[u8] = unsafe {
        let resource = FindResourceW(instance, id as usize as *const u16, RT_GROUP_ICON);
        if resource.is_null() {
            return Vec::new();
        }
        let handle = LoadResource(instance, resource);
        if handle.is_null() {
            return Vec::new();
        }
        let data = LockResource(handle) as *const u8;
        let length = SizeofResource(instance, resource) as usize;
        if data.is_null() || length < GROUP_HEADER_LEN {
            return Vec::new();
        }
        std::slice::from_raw_parts(data, length)
    };
    let count = u16_at(bytes, 4) as usize;
    (0..count)
        .filter_map(|index| {
            let at = GROUP_HEADER_LEN + index * GROUP_ENTRY_LEN;
            bytes.get(at).map(|width| match width {
                0 => 256,
                other => *other as u32,
            })
        })
        .collect()
}

/// The embedded width nearest to `wanted` pixels; a tie goes to the
/// larger image, which loses less when it is the wrong size.
fn nearest(widths: &[u32], wanted: u32) -> Option<u32> {
    widths
        .iter()
        .copied()
        .min_by_key(|width| (width.abs_diff(wanted), u32::MAX - width))
}

/// TigerSetup's brand mark at its own pixels: the embedded image whose
/// size is nearest to `wanted`, loaded at exactly that size so that
/// Windows neither scales nor interpolates it, and the size it has. Group
/// 2, or group 1 in an executable that has no second group, which the raw
/// engine is. `None` when the executable carries no icon at all.
pub fn brand_native(instance: HINSTANCE, wanted: i32) -> Option<(HICON, i32)> {
    let wanted = wanted.max(1) as u32;
    for id in [BRAND_ICON_ID, EXECUTABLE_ICON_ID] {
        let Some(width) = nearest(&group_widths(instance, id), wanted) else {
            continue;
        };
        let icon = resource(instance, id, width as i32);
        if !icon.is_null() {
            return Some((icon, width as i32));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory of three images with one byte of (invalid) image data
    /// each, which is all the parser looks at.
    fn ico(widths: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, widths.len() as u8, 0];
        let first_image = HEADER_LEN + widths.len() * ENTRY_LEN;
        for (index, width) in widths.iter().enumerate() {
            bytes.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&((first_image + index) as u32).to_le_bytes());
        }
        bytes.extend(std::iter::repeat_n(0xffu8, widths.len()));
        bytes
    }

    #[test]
    fn the_directory_lists_every_image_that_is_inside_the_file() {
        let entries = directory(&ico(&[16, 32, 48]));
        assert_eq!(
            entries.iter().map(|e| e.width).collect::<Vec<_>>(),
            vec![16, 32, 48]
        );
        assert_eq!(directory(&ico(&[0])).first().unwrap().width, 256);
    }

    #[test]
    fn a_file_that_is_not_an_icon_yields_nothing() {
        assert!(directory(b"").is_empty());
        assert!(directory(b"not an icon at all").is_empty());
        assert!(directory(&[0, 0, 2, 0, 1, 0]).is_empty());
        assert!(from_ico(b"MZ", 32).is_none());
    }

    #[test]
    fn an_entry_pointing_outside_the_file_is_ignored() {
        let mut bytes = ico(&[16, 32]);
        let second = HEADER_LEN + ENTRY_LEN;
        bytes[second + 12..second + 16].copy_from_slice(&9999u32.to_le_bytes());
        assert_eq!(
            directory(&bytes)
                .iter()
                .map(|e| e.width)
                .collect::<Vec<_>>(),
            vec![16]
        );
    }

    #[test]
    fn the_nearest_embedded_size_is_drawn_natively() {
        // The sizes TigerSetup's own icon carries, against the 24-dip mark at
        // the four validated scales: 24, 30, 36 and 48 pixels.
        let widths = [64, 48, 32, 24, 20, 16];
        assert_eq!(nearest(&widths, 24), Some(24));
        assert_eq!(nearest(&widths, 30), Some(32));
        assert_eq!(nearest(&widths, 36), Some(32));
        assert_eq!(nearest(&widths, 48), Some(48));
        // A tie goes to the larger image.
        assert_eq!(nearest(&[16, 32], 24), Some(32));
        assert_eq!(nearest(&[], 24), None);
    }

    #[test]
    fn the_closest_image_at_or_above_the_wanted_size_wins() {
        let entries = directory(&ico(&[16, 32, 48, 64]));
        assert_eq!(best(&entries, 32).unwrap().width, 32);
        assert_eq!(best(&entries, 40).unwrap().width, 48);
        assert_eq!(best(&entries, 20).unwrap().width, 32);
        // Nothing is large enough, so the largest is scaled down.
        assert_eq!(best(&entries, 256).unwrap().width, 64);
        assert!(best(&[], 32).is_none());
    }
}
