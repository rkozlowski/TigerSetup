//! What TigerSetup 0.7.1 decides about a file before any codec runs
//! (`crates/tigersetup-format/src/payload.rs`: the format-signature list, the
//! extension list, the sampled probe with its constants), reproduced here so
//! the spike can measure the decision rather than the product, plus the
//! semantic file families the layout experiments group by.

use std::io::Write;

/// The 0.7.1 probe constants, verbatim.
pub const PROBE_THRESHOLD: usize = 1 << 20;
pub const PROBE_SLICE: usize = 64 * 1024;
pub const PROBE_SLICES: usize = 3;
pub const PROBE_INCOMPRESSIBLE_RATIO: f64 = 0.98;
pub const PROBE_LEVEL: u32 = 1;

/// Why 0.7.1 stores a file without deflating it, or that it deflates it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Stored: its own format signature or extension says it is compressed.
    Signature,
    /// Stored: the sampled probe found nothing to gain.
    Probe,
    /// Handed to DEFLATE (and stored after all if that is not smaller).
    Deflate,
}

/// The semantic family a file is grouped by in the layout experiments.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    /// Text, configuration, source, scripts, markup, data-as-text.
    Text,
    /// .NET assemblies (CLR header present), satellite resource assemblies,
    /// resource files, localization catalogs.
    Managed,
    /// Native PE images: EXE, DLL, OCX, SYS, PYD, MUI, ...
    Native,
    /// Debug symbols and maps.
    Symbols,
    /// Fonts, icons, cursors, bitmaps, sounds and other uncompressed media.
    Resources,
    /// Everything else that is not known to be compressed already.
    Other,
    /// Already-compressed content by 0.7.1's signature/extension rule.
    Precompressed,
}

impl Family {
    pub fn name(self) -> &'static str {
        match self {
            Family::Text => "text",
            Family::Managed => "managed",
            Family::Native => "native",
            Family::Symbols => "symbols",
            Family::Resources => "resources",
            Family::Other => "other",
            Family::Precompressed => "precompressed",
        }
    }
}

/// `is_precompressed` from payload.rs 0.7.1, verbatim.
pub fn is_precompressed(name: &str, bytes: &[u8]) -> bool {
    const SIGNATURES: &[&[u8]] = &[
        b"PK\x03\x04",
        b"PK\x05\x06",
        b"\x1f\x8b",
        b"\xfd7zXZ\x00",
        b"\x28\xb5\x2f\xfd",
        b"7z\xbc\xaf\x27\x1c",
        b"Rar!",
        b"BZh",
        b"MSCF",
        b"\x89PNG\r\n\x1a\n",
        b"\xff\xd8\xff",
        b"GIF8",
        b"OggS",
        b"fLaC",
        b"ID3",
        b"\x1aE\xdf\xa3",
        b"wOFF",
        b"wOF2",
    ];
    if SIGNATURES.iter().any(|prefix| bytes.starts_with(prefix)) {
        return true;
    }
    if bytes.starts_with(b"RIFF") && matches!(bytes.get(8..12), Some(b"WEBP")) {
        return true;
    }
    if matches!(bytes.get(4..8), Some(b"ftyp")) {
        return true;
    }
    const EXTENSIONS: &[&str] = &[
        "7z", "avi", "br", "bz2", "cab", "flac", "gif", "gz", "jpeg", "jpg", "lz4", "lzma", "m4a",
        "m4v", "mkv", "mov", "mp3", "mp4", "nupkg", "ogg", "opus", "png", "rar", "vsix", "webm",
        "webp", "woff", "woff2", "xz", "zip", "zst",
    ];
    matches!(extension(name), Some(ext) if EXTENSIONS.contains(&ext.as_str()))
}

pub fn extension(name: &str) -> Option<String> {
    let file = name.rsplit('/').next().unwrap_or(name);
    file.rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext.to_ascii_lowercase())
}

pub fn deflated_size(bytes: &[u8], level: u32) -> u64 {
    let mut encoder =
        flate2::write::DeflateEncoder::new(CountingSink(0), flate2::Compression::new(level));
    encoder.write_all(bytes).expect("counting sink");
    encoder.finish().expect("counting sink").0
}

/// The 0.7.1 probe: three 64 KiB slices from the start, middle and end,
/// deflated at level 1. Returns the observed ratio and whether the product
/// calls the file incompressible.
pub fn probe(bytes: &[u8]) -> Option<(f64, bool)> {
    if bytes.len() < PROBE_THRESHOLD {
        return None;
    }
    let mut sampled = 0usize;
    let mut produced = 0u64;
    for index in 0..PROBE_SLICES {
        let Some(slice) = probe_slice(bytes, index) else {
            continue;
        };
        sampled += slice.len();
        produced += deflated_size(slice, PROBE_LEVEL);
    }
    if sampled == 0 {
        return None;
    }
    let ratio = produced as f64 / sampled as f64;
    Some((ratio, ratio >= PROBE_INCOMPRESSIBLE_RATIO))
}

fn probe_slice(bytes: &[u8], index: usize) -> Option<&[u8]> {
    let length = bytes.len();
    let span = PROBE_SLICE.min(length);
    let last = PROBE_SLICES - 1;
    let start = match index {
        0 => 0,
        i if i == last => length - span,
        i => (length - span) * i / last,
    };
    bytes.get(start..start + span)
}

pub fn decide(name: &str, bytes: &[u8]) -> (Decision, Option<f64>) {
    if bytes.is_empty() || is_precompressed(name, bytes) {
        return (Decision::Signature, None);
    }
    match probe(bytes) {
        Some((ratio, true)) => (Decision::Probe, Some(ratio)),
        Some((ratio, false)) => (Decision::Deflate, Some(ratio)),
        None => (Decision::Deflate, None),
    }
}

/// What kind of PE image the bytes are, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pe {
    Native,
    Managed,
}

pub fn pe_kind(bytes: &[u8]) -> Option<Pe> {
    if bytes.len() < 0x40 || &bytes[0..2] != b"MZ" {
        return None;
    }
    let e_lfanew = u32::from_le_bytes(bytes[0x3c..0x40].try_into().ok()?) as usize;
    if bytes.get(e_lfanew..e_lfanew + 4)? != b"PE\0\0" {
        return None;
    }
    let optional = e_lfanew + 24;
    let magic = u16::from_le_bytes(bytes.get(optional..optional + 2)?.try_into().ok()?);
    let directories = match magic {
        0x10b => optional + 96,
        0x20b => optional + 112,
        _ => return Some(Pe::Native),
    };
    // Data directory 14 is the CLR runtime header.
    let clr = directories + 14 * 8;
    let rva = u32::from_le_bytes(bytes.get(clr..clr + 4)?.try_into().ok()?);
    let size = u32::from_le_bytes(bytes.get(clr + 4..clr + 8)?.try_into().ok()?);
    Some(if rva != 0 && size != 0 {
        Pe::Managed
    } else {
        Pe::Native
    })
}

/// Extensions whose family the extension alone settles. Anything else is
/// decided by looking at the bytes: a PE header, or a text sniff.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "md",
    "markdown",
    "xml",
    "json",
    "toml",
    "ini",
    "cfg",
    "conf",
    "config",
    "html",
    "htm",
    "css",
    "scss",
    "less",
    "js",
    "mjs",
    "cjs",
    "ts",
    "jsx",
    "tsx",
    "map",
    "py",
    "pyi",
    "lua",
    "ps1",
    "psm1",
    "psd1",
    "cmd",
    "bat",
    "sh",
    "bash",
    "csv",
    "tsv",
    "yml",
    "yaml",
    "svg",
    "po",
    "pot",
    "properties",
    "manifest",
    "nsh",
    "nsi",
    "iss",
    "rst",
    "log",
    "sql",
    "qml",
    "tcl",
    "tk",
    "h",
    "c",
    "cpp",
    "hpp",
    "hxx",
    "cxx",
    "cc",
    "cs",
    "vb",
    "java",
    "rb",
    "pl",
    "pm",
    "php",
    "vue",
    "rc",
    "def",
    "idl",
    "proto",
    "pem",
    "crt",
    "isl",
    "lng",
    "lang",
    "inf",
    "reg",
    "url",
    "tex",
    "vim",
    "el",
    "plist",
    "xsd",
    "xsl",
    "dtd",
    "ui",
    "glade",
    "desktop",
    "resx",
    "xaml",
    "wxs",
    "tmlanguage",
    "snippets",
    "code-snippets",
    "ipynb",
    "gradle",
    "kt",
    "swift",
    "go",
    "rs",
    "diff",
    "patch",
    "jsonc",
    "json5",
    "webmanifest",
    "adoc",
    "1",
    "3",
    "5",
    "7",
    "8",
];
const SYMBOL_EXTENSIONS: &[&str] = &["pdb", "dbg", "sym", "ilk", "exp"];
const RESOURCE_EXTENSIONS: &[&str] = &[
    "ttf", "otf", "ttc", "fon", "pfb", "pfa", "afm", "ico", "cur", "ani", "bmp", "dib", "wav",
    "aiff", "aif", "au", "mid", "midi", "icns", "tif", "tiff", "tga", "pcx", "pbm", "pgm", "ppm",
    "pnm", "xbm", "xpm", "psd", "xcf", "theme", "msstyles",
];
const MANAGED_EXTENSIONS: &[&str] = &["resources", "baml"];

/// The family for a file, from its extension and — for PE images and
/// extension-less files — its leading bytes.
pub fn family(name: &str, head: &[u8]) -> Family {
    if is_precompressed(name, head) {
        return Family::Precompressed;
    }
    if let Some(pe) = pe_kind(head) {
        return match pe {
            Pe::Managed => Family::Managed,
            Pe::Native => Family::Native,
        };
    }
    let file = name.rsplit('/').next().unwrap_or(name).to_ascii_lowercase();
    if file.ends_with(".resources.dll") {
        return Family::Managed;
    }
    match extension(name).as_deref() {
        Some(ext) if TEXT_EXTENSIONS.contains(&ext) => Family::Text,
        Some(ext) if SYMBOL_EXTENSIONS.contains(&ext) => Family::Symbols,
        Some(ext) if RESOURCE_EXTENSIONS.contains(&ext) => Family::Resources,
        Some(ext) if MANAGED_EXTENSIONS.contains(&ext) => Family::Managed,
        Some("mo") | Some("qm") | Some("mui") => Family::Managed,
        Some(_) => {
            if looks_like_text(head) {
                Family::Text
            } else {
                Family::Other
            }
        }
        None => {
            if looks_like_text(head) {
                Family::Text
            } else {
                Family::Other
            }
        }
    }
}

/// No NUL byte in the first 4 KiB and mostly printable: a text file with an
/// extension the list above does not know (or none at all).
fn looks_like_text(head: &[u8]) -> bool {
    let sample = &head[..head.len().min(4096)];
    if sample.is_empty() || sample.contains(&0) {
        return false;
    }
    let printable = sample
        .iter()
        .filter(|&&b| b >= 0x20 || b == b'\n' || b == b'\r' || b == b'\t')
        .count();
    printable * 100 / sample.len() >= 95
}

struct CountingSink(u64);

impl Write for CountingSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
