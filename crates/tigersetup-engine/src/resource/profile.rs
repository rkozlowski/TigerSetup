//! The PowerShell profile block resource: tab completion for an installed
//! command, activated by a marked block in the CurrentUserAllHosts profile
//! of each PowerShell edition (`TigerSetup-Design.md` §5.15).
//!
//! ```powershell
//! # >>> TigerSetup completion: tool (Vendor.Tool) >>>
//! if (Test-Path -LiteralPath 'C:\...\completion\tool.ps1') { . 'C:\...\completion\tool.ps1' }
//! # <<< TigerSetup completion: tool (Vendor.Tool) <<<
//! ```
//!
//! The block is the only thing TigerSetup owns in a profile, and the rules
//! that keep it so each have a test:
//!
//! - everything outside the markers stays byte for byte as it was: the
//!   encoding and its byte-order mark, the line-ending style, the user's own
//!   lines, another application's blocks;
//! - the block is ASCII whatever the path, so it reads the same in every
//!   encoding a profile can have and in both editions — Windows PowerShell
//!   5.1 reads a file without a byte-order mark as ANSI, PowerShell 7 as
//!   UTF-8 — and a non-ASCII character of the path is written as
//!   `[char]0x...`;
//! - an existing block is replaced in place; a new one is appended after a
//!   blank line, and exactly the line breaks added for it are recorded
//!   (`separator`), so that taking it away restores the file as it was;
//! - a start marker without its end marker is a damaged block, which is
//!   never edited;
//! - the `Test-Path` guard keeps a profile working when the script is gone
//!   before the block is — an uninstall that cannot reach the profile.
//!
//! Nothing here changes execution policy or reads one to decide anything:
//! a profile the policy does not run simply does not run.

use std::path::{Path, PathBuf};

use crate::win::fs;
use crate::{Error, Result};

/// The profile file of every edition, in its directory under Documents.
pub const PROFILE_FILE: &str = "profile.ps1";

/// The result code of a block operation that could not reach its profile —
/// read-only, locked, an unavailable cloud placeholder — and was skipped:
/// completion is not worth failing an installation over, and a block that was
/// not written is not owned.
pub const SKIPPED: &str = "profile_unavailable_skipped";

/// The directory of each edition's CurrentUserAllHosts profile under the
/// user's Documents folder: Windows PowerShell 5.1, then PowerShell 7.
pub const EDITION_DIRECTORIES: [&str; 2] = ["WindowsPowerShell", "PowerShell"];

/// The CurrentUserAllHosts profiles of both editions under `documents`.
pub fn profiles(documents: &Path) -> Vec<PathBuf> {
    EDITION_DIRECTORIES
        .iter()
        .map(|directory| documents.join(directory).join(PROFILE_FILE))
        .collect()
}

/// Whether a path has the shape of a CurrentUserAllHosts profile:
/// `...\WindowsPowerShell\profile.ps1` or `...\PowerShell\profile.ps1`.
/// Nothing else is ever written as a profile, however a stored row names it.
pub fn is_profile_path(path: &Path) -> bool {
    path.is_absolute()
        && path
            .file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(PROFILE_FILE))
        && path
            .parent()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                EDITION_DIRECTORIES
                    .iter()
                    .any(|edition| edition.eq_ignore_ascii_case(name))
            })
}

/// The name a block's markers carry: the command and the product, so two
/// products that offer the same command never touch each other's block.
pub fn marker(command: &str, product_id: &str) -> String {
    format!("{command} ({product_id})")
}

fn start_line(marker: &str) -> String {
    format!("# >>> TigerSetup completion: {marker} >>>")
}

fn end_line(marker: &str) -> String {
    format!("# <<< TigerSetup completion: {marker} <<<")
}

/// The block for a script, its lines joined by `\n` — the form it is
/// recorded and compared in; a profile gets it in its own line endings.
pub fn block(marker: &str, script: &Path) -> String {
    let script = literal(&script.display().to_string());
    [
        start_line(marker),
        format!("if (Test-Path -LiteralPath {script}) {{ . {script} }}"),
        end_line(marker),
    ]
    .join("\n")
}

/// A PowerShell expression for `text` made of ASCII only: a single-quoted
/// literal, or — when the text has a character outside printable ASCII — a
/// parenthesized concatenation of single-quoted ASCII runs and
/// `[char]0x....` UTF-16 code units. It always starts with a string, so `+`
/// concatenates. A single quote is doubled, as PowerShell reads it.
pub fn literal(text: &str) -> String {
    let printable = |unit: u16| (0x20..0x7f).contains(&unit);
    let units: Vec<u16> = text.encode_utf16().collect();
    if units.iter().all(|unit| printable(*unit)) {
        return format!("'{}'", text.replace('\'', "''"));
    }
    let mut pieces: Vec<String> = Vec::new();
    let mut run = String::new();
    for unit in units {
        if printable(unit) {
            let c = unit as u8 as char;
            if c == '\'' {
                run.push_str("''");
            } else {
                run.push(c);
            }
        } else {
            if !run.is_empty() || pieces.is_empty() {
                pieces.push(format!("'{run}'"));
                run.clear();
            }
            pieces.push(format!("[char]0x{unit:04X}"));
        }
    }
    if !run.is_empty() {
        pieces.push(format!("'{run}'"));
    }
    format!("({})", pieces.join(" + "))
}

/// How a profile's bytes are text: one byte a unit — UTF-8 or ANSI, with or
/// without a UTF-8 byte-order mark, edited byte for byte — or UTF-16 with
/// its byte-order mark. The block is ASCII, so it is the same units in
/// every one of them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    Bytes,
    Utf16Le,
    Utf16Be,
}

/// A profile as units of text, the bytes in front of them (a UTF-16
/// byte-order mark) and a byte after them that is not a whole UTF-16 unit,
/// kept as it was.
#[derive(Debug, Clone)]
struct Text {
    encoding: Encoding,
    prefix: Vec<u8>,
    units: Vec<u16>,
    suffix: Vec<u8>,
}

const NEW_FILE_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

impl Text {
    fn decode(bytes: &[u8]) -> Text {
        let utf16 = |encoding: Encoding, rest: &[u8]| -> Text {
            let whole = rest.len() - rest.len() % 2;
            Text {
                encoding,
                prefix: bytes[..2].to_vec(),
                units: rest[..whole]
                    .chunks(2)
                    .map(|pair| match encoding {
                        Encoding::Utf16Le => u16::from_le_bytes([pair[0], pair[1]]),
                        _ => u16::from_be_bytes([pair[0], pair[1]]),
                    })
                    .collect(),
                suffix: rest[whole..].to_vec(),
            }
        };
        match bytes {
            [0xFF, 0xFE, rest @ ..] => utf16(Encoding::Utf16Le, rest),
            [0xFE, 0xFF, rest @ ..] => utf16(Encoding::Utf16Be, rest),
            _ => Text {
                encoding: Encoding::Bytes,
                prefix: Vec::new(),
                units: bytes.iter().map(|b| *b as u16).collect(),
                suffix: Vec::new(),
            },
        }
    }

    /// A new profile: UTF-8 with a byte-order mark, which both editions read
    /// as UTF-8.
    fn new_file() -> Text {
        Text {
            encoding: Encoding::Bytes,
            prefix: Vec::new(),
            units: NEW_FILE_BOM.iter().map(|b| *b as u16).collect(),
            suffix: Vec::new(),
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = self.prefix.clone();
        for unit in &self.units {
            match self.encoding {
                Encoding::Bytes => out.push(*unit as u8),
                Encoding::Utf16Le => out.extend_from_slice(&unit.to_le_bytes()),
                Encoding::Utf16Be => out.extend_from_slice(&unit.to_be_bytes()),
            }
        }
        out.extend_from_slice(&self.suffix);
        out
    }

    /// The units after a UTF-8 byte-order mark, where the text starts.
    fn body_start(&self) -> usize {
        match self.encoding {
            Encoding::Bytes if self.units.starts_with(&[0xEF, 0xBB, 0xBF]) => 3,
            _ => 0,
        }
    }

    /// The line break the file uses: CRLF where it has one, LF where it has
    /// only those, CRLF for a file with no line break at all.
    fn newline(&self) -> Vec<u16> {
        let units = &self.units;
        let lf = units.iter().position(|u| *u == b'\n' as u16);
        match lf {
            Some(index) if index > 0 && units[index - 1] == b'\r' as u16 => {
                vec![b'\r' as u16, b'\n' as u16]
            }
            Some(_) => vec![b'\n' as u16],
            None => vec![b'\r' as u16, b'\n' as u16],
        }
    }
}

fn units_of(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// The line spans of the text: `(start, content_end, line_end)`, where
/// `content_end` excludes the line break and `line_end` includes it.
fn lines(units: &[u16], from: usize) -> Vec<(usize, usize, usize)> {
    let mut out = Vec::new();
    let mut start = from;
    while start < units.len() {
        let lf = units[start..]
            .iter()
            .position(|u| *u == b'\n' as u16)
            .map(|offset| start + offset);
        let (content_end, line_end) = match lf {
            Some(index) => {
                let content_end = if index > start && units[index - 1] == b'\r' as u16 {
                    index - 1
                } else {
                    index
                };
                (content_end, index + 1)
            }
            None => (units.len(), units.len()),
        };
        out.push((start, content_end, line_end));
        start = line_end;
    }
    out
}

fn line_is(units: &[u16], start: usize, end: usize, wanted: &str) -> bool {
    let mut trimmed_end = end;
    while trimmed_end > start && matches!(units[trimmed_end - 1], 0x20 | 0x09) {
        trimmed_end -= 1;
    }
    units[start..trimmed_end] == units_of(wanted)[..]
}

/// Where a block is: from the first unit of its start marker to the end of
/// its end marker's line content, with the end of that line (its break
/// included).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    start: usize,
    end: usize,
    line_end: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Found {
    Absent,
    Present(Span),
    Damaged,
}

fn find(text: &Text, marker: &str) -> Found {
    let all = lines(&text.units, text.body_start());
    let start = start_line(marker);
    let end = end_line(marker);
    let Some(first) = all
        .iter()
        .position(|(s, e, _)| line_is(&text.units, *s, *e, &start))
    else {
        return Found::Absent;
    };
    match all[first + 1..]
        .iter()
        .find(|(s, e, _)| line_is(&text.units, *s, *e, &end))
    {
        Some((_, content_end, line_end)) => Found::Present(Span {
            start: all[first].0,
            end: *content_end,
            line_end: *line_end,
        }),
        None => Found::Damaged,
    }
}

fn damaged(path: &Path, marker: &str) -> Error {
    Error::new(
        "profile_block_damaged",
        format!(
            "{} has the start of the TigerSetup completion block for {marker} but not its end; it is not edited",
            path.display()
        ),
    )
}

/// The block the profile holds for `marker`, with `\n` line breaks, if any.
pub fn current_block(path: &Path, bytes: &[u8], marker: &str) -> Result<Option<String>> {
    let text = Text::decode(bytes);
    match find(&text, marker) {
        Found::Absent => Ok(None),
        Found::Damaged => Err(damaged(path, marker)),
        Found::Present(span) => {
            let content = String::from_utf16_lossy(&text.units[span.start..span.end]);
            Ok(Some(content.replace("\r\n", "\n")))
        }
    }
}

/// The line breaks an append puts in front of a block so that one blank
/// line separates it from what is there: none for an empty file, one break
/// for a file that ends with a line break, two for one that ends inside a
/// line, and none for a file that already ends with a blank line.
pub fn separator_for(bytes: Option<&[u8]>) -> String {
    let Some(bytes) = bytes else {
        return String::new();
    };
    let text = Text::decode(bytes);
    let body = &text.units[text.body_start()..];
    let newline = String::from_utf16_lossy(&text.newline());
    let lf = b'\n' as u16;
    let blank_at_end = body.ends_with(&[lf, lf]) || body.ends_with(&units_of("\n\r\n"));
    if body.is_empty() || blank_at_end {
        String::new()
    } else if body.ends_with(&[lf]) {
        newline
    } else {
        format!("{newline}{newline}")
    }
}

/// The profile with the block for `marker` set to `block`: replaced in
/// place where one is there, else appended after `separator_for`. Returns
/// the bytes, or `None` when the profile already holds exactly this block.
pub fn with_block(
    path: &Path,
    bytes: Option<&[u8]>,
    marker: &str,
    block: &str,
) -> Result<Option<Vec<u8>>> {
    let mut text = match bytes {
        Some(bytes) => Text::decode(bytes),
        None => Text::new_file(),
    };
    let newline = text.newline();
    let lines_of_block: Vec<u16> = {
        let mut out = Vec::new();
        for (index, line) in block.split('\n').enumerate() {
            if index > 0 {
                out.extend_from_slice(&newline);
            }
            out.extend(units_of(line));
        }
        out
    };
    match find(&text, marker) {
        Found::Damaged => Err(damaged(path, marker)),
        Found::Present(span) => {
            if text.units[span.start..span.end] == lines_of_block[..] {
                return Ok(None);
            }
            text.units.splice(span.start..span.end, lines_of_block);
            Ok(Some(text.encode()))
        }
        Found::Absent => {
            let separator = units_of(&separator_for(bytes));
            text.units.extend(separator);
            text.units.extend(lines_of_block);
            text.units.extend_from_slice(&newline);
            Ok(Some(text.encode()))
        }
    }
}

/// The profile without the block for `marker`, and without the line breaks
/// that were added in front of it (`separator`) where they are still there
/// as a blank line before it. `None` when the profile holds no such block.
pub fn without_block(
    path: &Path,
    bytes: &[u8],
    marker: &str,
    separator: &str,
) -> Result<Option<Vec<u8>>> {
    let mut text = Text::decode(bytes);
    let span = match find(&text, marker) {
        Found::Absent => return Ok(None),
        Found::Damaged => return Err(damaged(path, marker)),
        Found::Present(span) => span,
    };
    let mut start = span.start;
    let separator = units_of(separator);
    let body_start = text.body_start();
    if !separator.is_empty() {
        let before = &text.units[body_start..start];
        // Only a separator that still stands as a blank line in front of the
        // block goes: the file's last line break before it is the blank
        // line's own.
        let lf = b'\n' as u16;
        let blank_line_before =
            before.ends_with(&[lf, lf]) || before.ends_with(&units_of("\n\r\n"));
        if before.ends_with(&separator) && (blank_line_before || before.len() == separator.len()) {
            start -= separator.len();
        }
    }
    text.units.drain(start..span.line_end);
    Ok(Some(text.encode()))
}

/// Whether the bytes are a profile with nothing in it: empty, or only the
/// byte-order mark a new profile starts with.
pub fn is_empty(bytes: &[u8]) -> bool {
    bytes.is_empty() || bytes == NEW_FILE_BOM || bytes == [0xFF, 0xFE] || bytes == [0xFE, 0xFF]
}

// ---------------------------------------------------------------------------
// The profile on disk
// ---------------------------------------------------------------------------

fn refuse_unless_profile(path: &Path) -> Result<()> {
    if is_profile_path(path) {
        return Ok(());
    }
    Err(Error::new(
        "profile_path_invalid",
        format!(
            "{} is not a PowerShell CurrentUserAllHosts profile",
            path.display()
        ),
    ))
}

fn io(what: &str, path: &Path, err: std::io::Error) -> Error {
    Error::new(
        "profile_unavailable",
        format!("cannot {what} {}: {err}", path.display()),
    )
}

/// The profile's bytes, or `None` when there is no profile.
pub fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    refuse_unless_profile(path)?;
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(io("read", path, err)),
    }
}

/// Replaces the profile with `bytes`: written beside it, flushed, renamed
/// over it write-through, so an interruption leaves the old profile or the
/// new one and never half of either. A read-only profile refuses the
/// rename, and is left as it is.
fn write(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    let temp = fs::temp_path_for(path);
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&temp).map_err(|err| io("write", &temp, err))?;
        file.write_all(bytes)
            .map_err(|err| io("write", &temp, err))?;
        fs::flush(&file, &temp)?;
        drop(file);
        fs::rename_write_through(&temp, path)
            .map_err(|err| Error::new("profile_unavailable", err.message))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

/// What setting a block did to the profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Added {
    /// The profile changed.
    pub changed: bool,
    /// The profile file did not exist and was created.
    pub created_file: bool,
    /// The profile's directory did not exist and was created.
    pub created_directory: bool,
}

/// Sets the block: replaces the one there, or appends it, creating the
/// profile and its edition directory when they are missing. Idempotent.
pub fn add(path: &Path, marker: &str, block: &str) -> Result<Added> {
    refuse_unless_profile(path)?;
    let directory = path
        .parent()
        .ok_or_else(|| Error::new("profile_path_invalid", path.display().to_string()))?;
    let mut added = Added::default();
    if !directory.is_dir() {
        // Documents itself is the user's and is never created here.
        if !directory.parent().is_some_and(Path::is_dir) {
            return Err(Error::new(
                "profile_unavailable",
                format!("{} does not exist", directory.display()),
            ));
        }
        std::fs::create_dir(directory).map_err(|err| io("create", directory, err))?;
        added.created_directory = true;
    }
    let bytes = read(path)?;
    added.created_file = bytes.is_none();
    if let Some(next) = with_block(path, bytes.as_deref(), marker, block)? {
        write(path, &next)?;
        added.changed = true;
    }
    Ok(added)
}

/// Takes the block out of the profile, with the separator recorded for it.
/// A profile TigerSetup created (`delete_if_empty`) that holds nothing else
/// is deleted, and an edition directory TigerSetup created
/// (`remove_directory_if_empty`) is removed when that leaves it empty.
/// Returns whether the profile changed. Idempotent.
pub fn remove(
    path: &Path,
    marker: &str,
    separator: &str,
    delete_if_empty: bool,
    remove_directory_if_empty: bool,
) -> Result<bool> {
    refuse_unless_profile(path)?;
    let Some(bytes) = read(path)? else {
        return Ok(false);
    };
    let Some(next) = without_block(path, &bytes, marker, separator)? else {
        return Ok(false);
    };
    if delete_if_empty && is_empty(&next) {
        std::fs::remove_file(path).map_err(|err| io("delete", path, err))?;
        if remove_directory_if_empty && let Some(directory) = path.parent() {
            let _ = fs::remove_directory_if_empty(directory);
        }
    } else {
        write(path, &next)?;
    }
    Ok(true)
}

/// What a profile block's record carries about how it came to be: whether
/// TigerSetup created the profile file and its directory, and the line
/// breaks it put in front of the block. Recorded as `file=1;dir=0;sep=rn`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Origin {
    pub created_file: bool,
    pub created_directory: bool,
    pub separator: String,
}

/// The `restore_kind` a profile block's origin travels under.
pub const ORIGIN_KIND: &str = "profile_origin";

impl Origin {
    pub fn serialize(&self) -> String {
        let separator: String = self
            .separator
            .chars()
            .map(|c| match c {
                '\r' => 'r',
                _ => 'n',
            })
            .collect();
        format!(
            "file={};dir={};sep={separator}",
            self.created_file as u8, self.created_directory as u8
        )
    }

    pub fn parse(text: &str) -> Option<Origin> {
        let mut origin = Origin::default();
        for field in text.split(';') {
            let (name, value) = field.split_once('=')?;
            match name {
                "file" => origin.created_file = value == "1",
                "dir" => origin.created_directory = value == "1",
                "sep" => {
                    if !value.chars().all(|c| matches!(c, 'r' | 'n')) {
                        return None;
                    }
                    origin.separator = value
                        .chars()
                        .map(|c| if c == 'r' { '\r' } else { '\n' })
                        .collect();
                }
                _ => return None,
            }
        }
        Some(origin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MARKER: &str = "tool (Vendor.Tool)";

    fn script() -> PathBuf {
        PathBuf::from("C:\\Program Files\\Vendor Tool\\completion\\tool.ps1")
    }

    fn profile() -> PathBuf {
        PathBuf::from("C:\\Users\\x\\Documents\\PowerShell\\profile.ps1")
    }

    fn the_block() -> String {
        block(MARKER, &script())
    }

    fn add_to(bytes: Option<&[u8]>) -> Vec<u8> {
        with_block(&profile(), bytes, MARKER, &the_block())
            .unwrap()
            .expect("the profile changes")
    }

    #[test]
    fn the_block_is_ascii_whatever_the_path() {
        assert_eq!(literal("C:\\a b\\t.ps1"), "'C:\\a b\\t.ps1'");
        assert_eq!(literal("C:\\O'Brien\\t.ps1"), "'C:\\O''Brien\\t.ps1'");
        let polish = literal("C:\\Użytkownicy\\Zażółć\\t.ps1");
        assert!(polish.is_ascii(), "{polish}");
        assert_eq!(
            polish,
            "('C:\\U' + [char]0x017C + 'ytkownicy\\Za' + [char]0x017C + [char]0x00F3 + [char]0x0142 + [char]0x0107 + '\\t.ps1')"
        );
        // A path that starts with a non-ASCII character still starts with a
        // string, so `+` concatenates rather than adds.
        assert!(literal("Ż\\t.ps1").starts_with("('' + [char]0x017B"));
        // A character outside the Basic Multilingual Plane is its two code
        // units.
        assert!(literal("C:\\😀").contains("[char]0xD83D + [char]0xDE00"));
        let block = block(MARKER, Path::new("C:\\Zażółć\\completion\\tool.ps1"));
        assert!(block.is_ascii());
        assert_eq!(block.lines().count(), 3);
    }

    #[test]
    fn a_new_profile_is_utf8_with_a_byte_order_mark_and_crlf() {
        let bytes = add_to(None);
        assert!(bytes.starts_with(&NEW_FILE_BOM));
        let text = String::from_utf8(bytes[3..].to_vec()).unwrap();
        assert_eq!(text, format!("{}\r\n", the_block().replace('\n', "\r\n")));
        assert_eq!(
            current_block(&profile(), &bytes, MARKER)
                .unwrap()
                .as_deref(),
            Some(the_block().as_str())
        );
        // Taking it out leaves a profile with nothing in it.
        let back = without_block(&profile(), &bytes, MARKER, "")
            .unwrap()
            .unwrap();
        assert!(is_empty(&back));
    }

    #[test]
    fn everything_outside_the_block_is_kept_byte_for_byte() {
        let cases: Vec<(&str, Vec<u8>)> = vec![
            (
                "CRLF, final break",
                b"Set-Alias ll Get-ChildItem\r\n".to_vec(),
            ),
            ("LF, final break", b"Set-Alias ll Get-ChildItem\n".to_vec()),
            ("no final break", b"Set-Alias ll Get-ChildItem".to_vec()),
            ("ends with a blank line", b"$x = 1\r\n\r\n".to_vec()),
            (
                "UTF-8 BOM, Polish",
                [&NEW_FILE_BOM[..], "# zażółć gęślą jaźń\r\n".as_bytes()].concat(),
            ),
            (
                "ANSI (1250), Polish",
                vec![b'#', b' ', 0xBF, 0xF3, 0xB3, b'\r', b'\n'],
            ),
        ];
        for (name, original) in cases {
            let added = add_to(Some(&original));
            assert!(
                added.starts_with(&original),
                "{name}: the user's bytes stay in front"
            );
            let separator = separator_for(Some(&original));
            let back = without_block(&profile(), &added, MARKER, &separator)
                .unwrap()
                .unwrap();
            assert_eq!(back, original, "{name}: removal restores the file exactly");
            assert_eq!(
                with_block(&profile(), Some(&added), MARKER, &the_block()).unwrap(),
                None,
                "{name}: setting the same block again changes nothing"
            );
        }
    }

    #[test]
    fn the_line_ending_style_and_the_blank_line_follow_the_file() {
        let lf = add_to(Some(b"$a = 1\n"));
        let text = String::from_utf8(lf).unwrap();
        assert!(!text.contains('\r'), "an LF profile gets LF: {text:?}");
        assert!(text.starts_with("$a = 1\n\n# >>> TigerSetup"), "{text:?}");
        let joined = String::from_utf8(add_to(Some(b"$a = 1"))).unwrap();
        assert!(joined.starts_with("$a = 1\r\n\r\n# >>>"), "{joined:?}");
        let blank = String::from_utf8(add_to(Some(b"$a = 1\r\n\r\n"))).unwrap();
        assert!(blank.starts_with("$a = 1\r\n\r\n# >>>"), "{blank:?}");
        assert_eq!(separator_for(Some(b"$a = 1\r\n")), "\r\n");
        assert_eq!(separator_for(Some(b"$a = 1")), "\r\n\r\n");
        assert_eq!(separator_for(Some(b"$a = 1\r\n\r\n")), "");
        assert_eq!(separator_for(Some(b"")), "");
        assert_eq!(separator_for(None), "");
    }

    #[test]
    fn utf16_profiles_keep_their_encoding() {
        for (bom, little) in [([0xFFu8, 0xFE], true), ([0xFE, 0xFF], false)] {
            let mut original = bom.to_vec();
            for unit in "# zażółć\r\n".encode_utf16() {
                original.extend_from_slice(&if little {
                    unit.to_le_bytes()
                } else {
                    unit.to_be_bytes()
                });
            }
            let added = add_to(Some(&original));
            assert!(added.starts_with(&original));
            assert_eq!(added.len() % 2, 0);
            let block = current_block(&profile(), &added, MARKER).unwrap();
            assert_eq!(block.as_deref(), Some(the_block().as_str()));
            let back = without_block(&profile(), &added, MARKER, &separator_for(Some(&original)))
                .unwrap()
                .unwrap();
            assert_eq!(back, original);
        }
    }

    #[test]
    fn an_existing_block_is_replaced_in_place_and_others_are_untouched() {
        let old = block(MARKER, Path::new("C:\\Old\\completion\\tool.ps1")).replace('\n', "\r\n");
        let other = "# >>> TigerSetup completion: other (Vendor.Other) >>>\r\n. 'x'\r\n# <<< TigerSetup completion: other (Vendor.Other) <<<";
        let original = format!("# mine\r\n{old}\r\n{other}\r\n# after\r\n");
        let updated = String::from_utf8(add_to(Some(original.as_bytes()))).unwrap();
        assert_eq!(
            updated,
            format!(
                "# mine\r\n{}\r\n{other}\r\n# after\r\n",
                the_block().replace('\n', "\r\n")
            )
        );
        // Removing ours takes exactly its lines.
        let removed = without_block(&profile(), updated.as_bytes(), MARKER, "")
            .unwrap()
            .unwrap();
        assert_eq!(
            String::from_utf8(removed).unwrap(),
            format!("# mine\r\n{other}\r\n# after\r\n")
        );
        // The other product's block is not ours, by its marker.
        assert_eq!(
            current_block(&profile(), other.as_bytes(), MARKER).unwrap(),
            None
        );
    }

    #[test]
    fn a_damaged_block_is_never_edited() {
        let damaged =
            format!("# >>> TigerSetup completion: {MARKER} >>>\r\n. 'x'\r\n# user line\r\n");
        assert!(with_block(&profile(), Some(damaged.as_bytes()), MARKER, &the_block()).is_err());
        assert!(without_block(&profile(), damaged.as_bytes(), MARKER, "").is_err());
        assert_eq!(
            current_block(&profile(), damaged.as_bytes(), MARKER)
                .unwrap_err()
                .code,
            "profile_block_damaged"
        );
    }

    #[test]
    fn a_separator_the_user_already_took_away_is_not_taken_again() {
        // The user deleted the blank line in front of the block: removal takes
        // the block alone and leaves the user's line break.
        let text = format!("$a = 1\r\n{}\r\n", the_block().replace('\n', "\r\n"));
        let back = without_block(&profile(), text.as_bytes(), MARKER, "\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(back, b"$a = 1\r\n");
        // Lines the user added after the block stay.
        let text = format!(
            "$a = 1\r\n\r\n{}\r\n$b = 2\r\n",
            the_block().replace('\n', "\r\n")
        );
        let back = without_block(&profile(), text.as_bytes(), MARKER, "\r\n")
            .unwrap()
            .unwrap();
        assert_eq!(back, b"$a = 1\r\n$b = 2\r\n");
    }

    #[test]
    fn only_profile_paths_are_profiles() {
        assert!(is_profile_path(&profile()));
        assert!(is_profile_path(Path::new(
            "D:\\OneDrive\\Dokumenty\\WindowsPowerShell\\Profile.ps1"
        )));
        assert!(!is_profile_path(Path::new(
            "C:\\Windows\\System32\\drivers\\etc\\hosts"
        )));
        assert!(!is_profile_path(Path::new(
            "C:\\Users\\x\\Documents\\profile.ps1"
        )));
        assert!(!is_profile_path(Path::new("PowerShell\\profile.ps1")));
        assert_eq!(
            profiles(Path::new("C:\\D")),
            vec![
                PathBuf::from("C:\\D\\WindowsPowerShell\\profile.ps1"),
                PathBuf::from("C:\\D\\PowerShell\\profile.ps1")
            ]
        );
    }

    #[test]
    fn the_origin_round_trips() {
        let origin = Origin {
            created_file: true,
            created_directory: false,
            separator: "\r\n\r\n".into(),
        };
        assert_eq!(origin.serialize(), "file=1;dir=0;sep=rnrn");
        assert_eq!(Origin::parse(&origin.serialize()), Some(origin));
        assert_eq!(Origin::parse("file=0;dir=0;sep="), Some(Origin::default()));
        assert_eq!(Origin::parse("sep=x"), None);
    }

    /// The editions this machine has: Windows PowerShell 5.1 always, and
    /// PowerShell 7 where it is installed.
    fn editions() -> Vec<PathBuf> {
        let mut found = Vec::new();
        if let Some(system) = crate::win::env::windows_directory() {
            found.push(system.join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"));
        }
        if let Ok(output) = std::process::Command::new("where.exe")
            .arg("pwsh.exe")
            .output()
            && let Some(first) = String::from_utf8_lossy(&output.stdout).lines().next()
        {
            found.push(PathBuf::from(first.trim()));
        }
        found
    }

    #[test]
    fn the_block_loads_its_script_in_both_editions_from_any_profile_encoding() {
        let root = tempfile::tempdir().unwrap();
        // A non-ASCII, quote-bearing directory: what the block must spell in
        // ASCII and both editions must still read.
        let directory = root.path().join("Zażółć O'Brien");
        std::fs::create_dir_all(&directory).unwrap();
        let script = directory.join("tool.ps1");
        std::fs::write(&script, b"$global:TigerSetupProbe = 'loaded'\r\n").unwrap();
        let block = block(MARKER, &script).replace('\n', "\r\n");
        let mut utf16 = vec![0xFF, 0xFE];
        for unit in format!("$before = 1\r\n{block}\r\n").encode_utf16() {
            utf16.extend_from_slice(&unit.to_le_bytes());
        }
        let encodings: Vec<(&str, Vec<u8>)> = vec![
            (
                "utf-8 bom",
                [&NEW_FILE_BOM[..], format!("{block}\r\n").as_bytes()].concat(),
            ),
            ("no bom", format!("$before = 1\n{block}\n").into_bytes()),
            ("utf-16le", utf16),
        ];
        let editions = editions();
        assert!(!editions.is_empty());
        for (name, bytes) in encodings {
            let profile = root.path().join(format!("{}.ps1", name.replace(' ', "-")));
            std::fs::write(&profile, &bytes).unwrap();
            for shell in &editions {
                // Bypass is this child's own process scope, the test's way of
                // loading a profile under any machine policy; nothing changes
                // a policy anywhere.
                let output = std::process::Command::new(shell)
                    .args([
                        "-NoLogo",
                        "-NoProfile",
                        "-NonInteractive",
                        "-ExecutionPolicy",
                        "Bypass",
                        "-Command",
                    ])
                    .arg(format!(
                        ". '{}'; Write-Output $global:TigerSetupProbe",
                        profile.display().to_string().replace('\'', "''")
                    ))
                    .output()
                    .unwrap();
                let stdout = String::from_utf8_lossy(&output.stdout);
                assert_eq!(
                    stdout.trim(),
                    "loaded",
                    "{name} profile in {}: {stdout} {}",
                    shell.display(),
                    String::from_utf8_lossy(&output.stderr)
                );
            }
        }
        // With the script gone, the guard keeps the profile quiet.
        std::fs::remove_file(&script).unwrap();
        let profile = root.path().join("utf-8-bom.ps1");
        for shell in &editions {
            let output = std::process::Command::new(shell)
                .args([
                    "-NoLogo",
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                ])
                .arg(format!(". '{}'; 'quiet'", profile.display()))
                .output()
                .unwrap();
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "quiet");
            assert!(
                output.stderr.is_empty(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }

    #[test]
    fn add_and_remove_on_disk_create_and_take_away_only_what_they_made() {
        let documents = tempfile::tempdir().unwrap();
        let profile = documents.path().join("PowerShell").join(PROFILE_FILE);
        let added = add(&profile, MARKER, &the_block()).unwrap();
        assert_eq!(
            added,
            Added {
                changed: true,
                created_file: true,
                created_directory: true
            }
        );
        assert!(!add(&profile, MARKER, &the_block()).unwrap().changed);
        assert!(remove(&profile, MARKER, "", true, true).unwrap());
        assert!(!profile.exists() && !profile.parent().unwrap().exists());
        assert!(!remove(&profile, MARKER, "", true, true).unwrap());
        // A profile with the user's own line stays, with that line.
        std::fs::create_dir_all(profile.parent().unwrap()).unwrap();
        std::fs::write(&profile, b"$mine = 1\r\n").unwrap();
        let added = add(&profile, MARKER, &the_block()).unwrap();
        assert!(!added.created_file && !added.created_directory);
        assert!(remove(&profile, MARKER, "\r\n", true, true).unwrap());
        assert_eq!(std::fs::read(&profile).unwrap(), b"$mine = 1\r\n");
        // Documents itself is never created.
        let elsewhere = documents
            .path()
            .join("Missing")
            .join("PowerShell")
            .join(PROFILE_FILE);
        assert_eq!(
            add(&elsewhere, MARKER, &the_block()).unwrap_err().code,
            "profile_unavailable"
        );
        // A read-only profile is left as it is.
        let read_only = std::fs::metadata(&profile).unwrap().permissions();
        let mut permissions = read_only.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&profile, permissions).unwrap();
        assert_eq!(
            add(&profile, MARKER, &the_block()).unwrap_err().code,
            "profile_unavailable"
        );
        assert_eq!(std::fs::read(&profile).unwrap(), b"$mine = 1\r\n");
        let mut writable = std::fs::metadata(&profile).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        writable.set_readonly(false);
        std::fs::set_permissions(&profile, writable).unwrap();
        // Nothing but a profile is ever written.
        assert_eq!(
            add(&documents.path().join("x.ps1"), MARKER, &the_block())
                .unwrap_err()
                .code,
            "profile_path_invalid"
        );
    }
}
