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
    /// Extension, then the file name without it, then path: same-named
    /// files of one kind side by side whatever directory holds them.
    ExtBasenamePath,
    /// The static type family (`type_of`), then path.
    TypePath,
    /// The static type family, then the file name without its extension,
    /// then the extension, then path.
    TypeBasenameExtPath,
    /// The static type family, then the extension, then the file name
    /// without it, then path.
    TypeExtBasenamePath,
}

/// The broad, static compression family of an entry, from its lower-cased
/// extension alone: no content inspection. The families are ordered as
/// the stream carries them.
pub fn type_of(ext: Option<&str>) -> usize {
    const TEXT: &[&str] = &[
        "txt", "md", "json", "xml", "yaml", "yml", "toml", "ini", "cfg", "conf", "html", "htm",
        "css", "js", "mjs", "cjs", "ts", "cts", "mts", "jsx", "tsx", "svg", "csv", "log", "sh",
        "ps1", "psm1", "psd1", "bat", "cmd", "py", "pyi", "rb", "pl", "pm", "pod", "lua", "sql",
        "h", "c", "cpp", "hpp", "cs", "vb", "java", "properties", "manifest", "config", "resx",
        "xaml", "po", "pot", "adoc", "rst", "tex", "vim", "tcl", "tk", "lang", "nls", "map",
        "rtf", "pem", "crt", "license", "vcxproj", "csproj", "props", "targets", "nuspec",
    ];
    const MODULE: &[&str] = &[
        "exe", "dll", "sys", "ocx", "drv", "cpl", "scr", "com", "mui", "node", "pyd", "winmd",
        "efi",
    ];
    const IMAGE: &[&str] = &[
        "png", "jpg", "jpeg", "gif", "bmp", "ico", "cur", "ani", "webp", "tif", "tiff", "xcf",
    ];
    const COMPRESSED: &[&str] = &[
        "zip", "7z", "gz", "tgz", "bz2", "xz", "zst", "rar", "cab", "jar", "whl", "nupkg", "mp3",
        "mp4", "m4a", "ogg", "webm", "mkv", "avi", "chm",
    ];
    const FONT: &[&str] = &["ttf", "otf", "woff", "woff2", "eot", "fon"];
    let ext = ext.unwrap_or("");
    if TEXT.contains(&ext) {
        0
    } else if MODULE.contains(&ext) {
        1
    } else if IMAGE.contains(&ext) {
        2
    } else if COMPRESSED.contains(&ext) {
        3
    } else if FONT.contains(&ext) {
        4
    } else {
        5
    }
}

/// The file name without its extension, lower-cased.
fn stem_of(path: &str) -> String {
    let name = path.rsplit('/').next().unwrap_or(path);
    match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem.to_ascii_lowercase(),
        _ => name.to_ascii_lowercase(),
    }
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
            Layout::Path
            | Layout::ExtPath
            | Layout::ExtBasenamePath
            | Layout::TypePath
            | Layout::TypeBasenameExtPath
            | Layout::TypeExtBasenamePath => 0,
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
    // The sort key of an entry under the layout: the family rank (the
    // spike's semantic families or the static type), then the keys in the
    // layout's own order, then the path.
    let key = |f: &crate::inventory::FileEntry| -> (usize, String, String) {
        let ext = f.ext.clone().unwrap_or_default();
        match layout {
            Layout::Path => (0, String::new(), String::new()),
            Layout::ExtPath => (0, ext, String::new()),
            Layout::TextFirst | Layout::BinaryFirst | Layout::LargestFirst => {
                (rank(f.family), String::new(), String::new())
            }
            Layout::TextFirstExt => (rank(f.family), ext, String::new()),
            Layout::ExtBasenamePath => (0, ext, stem_of(&f.path)),
            Layout::TypePath => (type_of(f.ext.as_deref()), String::new(), String::new()),
            Layout::TypeBasenameExtPath => (type_of(f.ext.as_deref()), stem_of(&f.path), ext),
            Layout::TypeExtBasenamePath => (type_of(f.ext.as_deref()), ext, stem_of(&f.path)),
        }
    };
    files.sort_by(|a, b| {
        key(a)
            .cmp(&key(b))
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
