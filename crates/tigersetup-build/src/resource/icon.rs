//! Icon resources from the bytes of an `.ico` file. The file is a directory
//! of images; Windows stores the same thing as one `RT_ICON` resource per
//! image, bytes exactly as in the file (a PNG-compressed entry stays PNG),
//! and one `RT_GROUP_ICON` directory whose entries name the image resources
//! instead of file offsets. The order is the file's own.

use crate::{BuildError, Result};

const HEADER_LEN: usize = 6;
const FILE_ENTRY_LEN: usize = 16;
const GROUP_ENTRY_LEN: usize = 14;
const ICO_TYPE: u16 = 1;

/// One image of an icon: the 12 directory bytes the file and the resource
/// group share (`bWidth`, `bHeight`, `bColorCount`, `bReserved`, `wPlanes`,
/// `wBitCount`, `dwBytesInRes`) and the image bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub directory: [u8; 12],
    pub bytes: Vec<u8>,
}

impl Image {
    /// Pixels; the directory stores 0 for 256.
    pub fn width(&self) -> u32 {
        match self.directory[0] {
            0 => 256,
            other => other as u32,
        }
    }

    pub fn height(&self) -> u32 {
        match self.directory[1] {
            0 => 256,
            other => other as u32,
        }
    }

    pub fn bits(&self) -> u16 {
        u16::from_le_bytes([self.directory[6], self.directory[7]])
    }
}

fn u16_at(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The images of an `.ico` file, in the file's order. Every entry must lie
/// inside the file: an icon the builder cannot read whole is refused rather
/// than embedded in part.
pub fn parse(what: &str, bytes: &[u8]) -> Result<Vec<Image>> {
    let invalid = |detail: &str| BuildError::new("icon_invalid", format!("{what}: {detail}"));
    if bytes.len() < HEADER_LEN || u16_at(bytes, 0) != 0 || u16_at(bytes, 2) != ICO_TYPE {
        return Err(invalid("not an .ico file"));
    }
    let count = u16_at(bytes, 4) as usize;
    if count == 0 {
        return Err(invalid("the icon has no images"));
    }
    let mut images = Vec::with_capacity(count);
    for index in 0..count {
        let at = HEADER_LEN + index * FILE_ENTRY_LEN;
        if at + FILE_ENTRY_LEN > bytes.len() {
            return Err(invalid("the icon directory is truncated"));
        }
        let length = u32_at(bytes, at + 8) as usize;
        let offset = u32_at(bytes, at + 12) as usize;
        if length == 0
            || offset
                .checked_add(length)
                .is_none_or(|end| end > bytes.len())
        {
            return Err(invalid(&format!(
                "image {} lies outside the file",
                index + 1
            )));
        }
        let mut directory = [0u8; 12];
        directory.copy_from_slice(&bytes[at..at + 12]);
        images.push(Image {
            directory,
            bytes: bytes[offset..offset + length].to_vec(),
        });
    }
    Ok(images)
}

/// The `RT_GROUP_ICON` directory for images stored as `RT_ICON` resources
/// `first_id`, `first_id + 1`, … in the same order.
pub fn group(images: &[Image], first_id: u16) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + images.len() * GROUP_ENTRY_LEN);
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&ICO_TYPE.to_le_bytes());
    out.extend_from_slice(&(images.len() as u16).to_le_bytes());
    for (index, image) in images.iter().enumerate() {
        out.extend_from_slice(&image.directory);
        out.extend_from_slice(&(first_id + index as u16).to_le_bytes());
    }
    out
}

/// The image ids an `RT_GROUP_ICON` directory names, in its order, with the
/// directory bytes of each.
pub fn group_entries(directory: &[u8]) -> Result<Vec<([u8; 12], u16)>> {
    let invalid = |detail: &str| BuildError::new("icon_invalid", format!("icon group: {detail}"));
    if directory.len() < HEADER_LEN || u16_at(directory, 2) != ICO_TYPE {
        return Err(invalid("not an icon group directory"));
    }
    let count = u16_at(directory, 4) as usize;
    let mut entries = Vec::with_capacity(count);
    for index in 0..count {
        let at = HEADER_LEN + index * GROUP_ENTRY_LEN;
        if at + GROUP_ENTRY_LEN > directory.len() {
            return Err(invalid("the directory is truncated"));
        }
        let mut bytes = [0u8; 12];
        bytes.copy_from_slice(&directory[at..at + 12]);
        entries.push((bytes, u16_at(directory, at + 12)));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An icon of the given widths whose images are one byte each.
    pub fn ico(widths: &[u8]) -> Vec<u8> {
        let mut bytes = vec![0, 0, 1, 0, widths.len() as u8, 0];
        let first_image = HEADER_LEN + widths.len() * FILE_ENTRY_LEN;
        for (index, width) in widths.iter().enumerate() {
            bytes.extend_from_slice(&[*width, *width, 0, 0, 1, 0, 32, 0]);
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&((first_image + index) as u32).to_le_bytes());
        }
        bytes.extend((0..widths.len()).map(|i| 0xA0 + i as u8));
        bytes
    }

    #[test]
    fn the_images_come_out_in_file_order_with_their_bytes() {
        let images = parse("test", &ico(&[16, 0, 48])).unwrap();
        assert_eq!(
            images.iter().map(Image::width).collect::<Vec<_>>(),
            vec![16, 256, 48]
        );
        assert_eq!(images[1].bytes, vec![0xA1]);
        assert_eq!(images[0].bits(), 32);
        let group = group(&images, 4);
        assert_eq!(group.len(), HEADER_LEN + 3 * GROUP_ENTRY_LEN);
        assert_eq!(
            group_entries(&group)
                .unwrap()
                .iter()
                .map(|(_, id)| *id)
                .collect::<Vec<_>>(),
            vec![4, 5, 6]
        );
    }

    #[test]
    fn a_file_that_is_not_a_whole_icon_is_refused() {
        assert_eq!(parse("t", b"").unwrap_err().code, "icon_invalid");
        assert_eq!(
            parse("t", b"MZ not an icon").unwrap_err().code,
            "icon_invalid"
        );
        assert_eq!(
            parse("t", &[0, 0, 1, 0, 0, 0]).unwrap_err().code,
            "icon_invalid"
        );
        let mut truncated = ico(&[16, 32]);
        truncated.pop();
        assert_eq!(parse("t", &truncated).unwrap_err().code, "icon_invalid");
    }
}
