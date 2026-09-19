//! Which files go into the solid stream (the partition) and in what order
//! (the layout). Both are pure functions of the inventory, so a list can be
//! regenerated from the committed results.

use serde::{Deserialize, Serialize};

use crate::classify::{Decision, Family};
use crate::inventory::Inventory;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Partition {
    /// 0.7.1's decision: signature- and probe-stored files stay raw.
    Current,
    /// Only the signature rule keeps a file raw; probe-rejected files are
    /// admitted to the solid stream.
    Signature,
    /// Every non-empty file goes into the solid stream.
    All,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Layout {
    /// Byte order of the install-relative path: what 0.7.1 does.
    Path,
    /// Extension, then path: grouping with no semantics at all.
    ExtPath,
    /// Families: text, resources, other, symbols, managed, native, precompressed.
    TextFirst,
    /// The reverse: native, managed, symbols, other, resources, text, precompressed.
    BinaryFirst,
    /// Families by total bytes, largest first.
    LargestFirst,
    /// `text-first` groups, extension-then-path inside each group.
    TextFirstExt,
}

const TEXT_FIRST: [Family; 7] = [
    Family::Text,
    Family::Resources,
    Family::Other,
    Family::Symbols,
    Family::Managed,
    Family::Native,
    Family::Precompressed,
];
const BINARY_FIRST: [Family; 7] = [
    Family::Native,
    Family::Managed,
    Family::Symbols,
    Family::Other,
    Family::Resources,
    Family::Text,
    Family::Precompressed,
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileList {
    pub app: String,
    pub payload_root: String,
    pub partition: Partition,
    pub layout: Layout,
    pub admitted_files: u64,
    pub admitted_bytes: u64,
    pub raw_files: u64,
    pub raw_bytes: u64,
    /// The families in the order the stream carries them, with their bytes.
    pub group_order: Vec<(String, u64)>,
    pub files: Vec<String>,
}

pub fn build(inventory: &Inventory, partition: Partition, layout: Layout) -> FileList {
    let admitted = |decision: Decision| match partition {
        Partition::Current => decision == Decision::Deflate,
        Partition::Signature => decision != Decision::Signature,
        Partition::All => true,
    };
    let mut files: Vec<_> = inventory
        .files
        .iter()
        .filter(|f| f.bytes > 0 && admitted(f.decision))
        .collect();
    let raw: Vec<_> = inventory
        .files
        .iter()
        .filter(|f| f.bytes > 0 && !admitted(f.decision))
        .collect();

    let mut family_bytes = std::collections::BTreeMap::new();
    for f in &files {
        *family_bytes.entry(f.family).or_insert(0u64) += f.bytes;
    }
    let rank = |family: Family| -> usize {
        match layout {
            Layout::Path | Layout::ExtPath => 0,
            Layout::TextFirst | Layout::TextFirstExt => {
                TEXT_FIRST.iter().position(|&f| f == family).unwrap()
            }
            Layout::BinaryFirst => BINARY_FIRST.iter().position(|&f| f == family).unwrap(),
            Layout::LargestFirst => {
                let mut order: Vec<_> = family_bytes.iter().collect();
                order.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
                order.iter().position(|(f, _)| **f == family).unwrap()
            }
        }
    };
    let ext_key = |f: &crate::inventory::FileEntry| -> String {
        match layout {
            Layout::ExtPath | Layout::TextFirstExt => f.ext.clone().unwrap_or_default(),
            _ => String::new(),
        }
    };
    files.sort_by(|a, b| {
        rank(a.family)
            .cmp(&rank(b.family))
            .then_with(|| ext_key(a).cmp(&ext_key(b)))
            .then_with(|| a.path.as_bytes().cmp(b.path.as_bytes()))
    });

    let mut group_order: Vec<(String, u64)> = Vec::new();
    for f in &files {
        match group_order.last_mut() {
            Some((name, bytes)) if name == f.family.name() => *bytes += f.bytes,
            _ => group_order.push((f.family.name().to_string(), f.bytes)),
        }
    }

    FileList {
        app: inventory.app.clone(),
        payload_root: inventory.payload_root.clone(),
        partition,
        layout,
        admitted_files: files.len() as u64,
        admitted_bytes: files.iter().map(|f| f.bytes).sum(),
        raw_files: raw.len() as u64,
        raw_bytes: raw.iter().map(|f| f.bytes).sum(),
        group_order,
        files: files.iter().map(|f| f.path.clone()).collect(),
    }
}
