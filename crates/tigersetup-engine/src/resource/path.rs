//! The PATH entry resource: one directory appended as the last entry of the
//! scope's `Path` (`REG_EXPAND_SZ`), recorded by its exact raw text and its
//! normalized form. The rules, each with a test:
//!
//! - an equivalent entry that already exists is neither duplicated nor
//!   claimed;
//! - removal takes the exact raw text first, then a normalized match, and
//!   removes exactly one segment — every other segment, including an empty
//!   one from a trailing `;`, stays as it was;
//! - appending to a `Path` that ends in `;` keeps that trailing empty
//!   segment;
//! - a reinstall that keeps the entry leaves exactly one.
//!
//! The pure functions decide; [`add`] and [`remove`] read and write the
//! registry value and are idempotent.

use crate::win::registry::{self, Data, KeyPath, Roots};
use crate::{Error, Result};

pub const VALUE_NAME: &str = "Path";

/// The comparison form of an entry: trimmed, unquoted, environment
/// variables expanded, forward slashes folded, trailing separators dropped
/// (a drive root keeps its one), lower-cased.
pub fn normalize(entry: &str) -> String {
    let trimmed = entry.trim().trim_matches('"').trim();
    let expanded = expand_environment(trimmed).replace('/', "\\");
    let mut out = expanded.trim_end_matches('\\').to_string();
    if out.len() == 2 && out.as_bytes()[1] == b':' {
        out.push('\\');
    }
    out.to_lowercase()
}

/// Expands `%VAR%` for comparison only, and deliberately not the way
/// `identity::expand_template` does: that one resolves a package's own
/// placeholders and refuses a name it does not know, because a bad template is
/// a broken package. This one reads `PATH` entries other software wrote, where
/// an unknown or unset variable is ordinary — so it leaves the text as it
/// stands rather than failing, and two spellings of an entry still compare
/// equal.
fn expand_environment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(value) => out.push_str(&value),
                    Err(_) => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The segments of a `Path` text, empty ones included.
pub fn split(text: &str) -> Vec<&str> {
    if text.is_empty() {
        Vec::new()
    } else {
        text.split(';').collect()
    }
}

/// The index of the segment to remove for an owned entry: the exact raw
/// text, else the first normalized match.
pub fn find_owned(text: &str, raw: &str, normalized: &str) -> Option<usize> {
    let segments = split(text);
    segments
        .iter()
        .position(|segment| *segment == raw)
        .or_else(|| {
            segments
                .iter()
                .position(|segment| !segment.is_empty() && normalize(segment) == normalized)
        })
}

/// Whether an equivalent of the entry is present.
pub fn contains_equivalent(text: &str, normalized: &str) -> bool {
    split(text)
        .iter()
        .any(|segment| !segment.is_empty() && normalize(segment) == normalized)
}

/// The text with `raw` appended as the last entry. A trailing empty segment
/// (`...;`) is kept trailing.
pub fn appended(text: &str, raw: &str) -> String {
    if text.is_empty() {
        raw.to_string()
    } else if text.ends_with(';') {
        format!("{text}{raw};")
    } else {
        format!("{text};{raw}")
    }
}

/// The text without the owned entry, or `None` when it holds none.
pub fn removed(text: &str, raw: &str, normalized: &str) -> Option<String> {
    let index = find_owned(text, raw, normalized)?;
    let mut segments = split(text);
    segments.remove(index);
    Some(segments.join(";"))
}

/// The current `Path` text of the environment key and whether the value
/// exists.
pub fn read(roots: &Roots, environment_key: &KeyPath) -> Result<(String, bool)> {
    match registry::read_value(roots, environment_key, VALUE_NAME)? {
        None => Ok((String::new(), false)),
        Some(Data::String(text)) | Some(Data::ExpandString(text)) => Ok((text, true)),
        Some(other) => Err(Error::new(
            "path_value_unreadable",
            format!(
                "{environment_key}\\{VALUE_NAME} is a {} value, not a string",
                other.kind_name()
            ),
        )),
    }
}

/// Writes the value, creating the environment key when the scope has none.
/// The key belongs to Windows: TigerSetup writes `Path` into it but never
/// owns or removes it.
fn write(roots: &Roots, environment_key: &KeyPath, text: &str) -> Result<()> {
    if !registry::key_exists(roots, environment_key)? {
        registry::create_key(roots, environment_key)?;
    }
    registry::write_value(
        roots,
        environment_key,
        VALUE_NAME,
        &Data::ExpandString(text.to_string()),
    )
}

/// Appends the entry unless an equivalent is present. Returns whether the
/// value changed.
pub fn add(roots: &Roots, environment_key: &KeyPath, raw: &str) -> Result<bool> {
    let (text, _) = read(roots, environment_key)?;
    if contains_equivalent(&text, &normalize(raw)) {
        return Ok(false);
    }
    write(roots, environment_key, &appended(&text, raw))?;
    Ok(true)
}

/// Removes the owned entry if present. With `delete_when_empty`, a `Path`
/// left empty is deleted (it did not exist before TigerSetup wrote it).
/// Returns whether the value changed.
pub fn remove(
    roots: &Roots,
    environment_key: &KeyPath,
    raw: &str,
    delete_when_empty: bool,
) -> Result<bool> {
    let (text, exists) = read(roots, environment_key)?;
    if !exists {
        return Ok(false);
    }
    let Some(next) = removed(&text, raw, &normalize(raw)) else {
        return Ok(false);
    };
    if next.is_empty() && delete_when_empty {
        registry::delete_value(roots, environment_key, VALUE_NAME)?;
    } else {
        write(roots, environment_key, &next)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "C:\\Users\\x\\AppData\\Local\\Programs\\TestApp\\bin";

    #[test]
    fn normalization_folds_the_equivalent_spellings() {
        let normalized = normalize(ENTRY);
        assert_eq!(normalize(&format!("{ENTRY}\\")), normalized);
        assert_eq!(normalize(&ENTRY.to_uppercase()), normalized);
        assert_eq!(normalize(&format!(" \"{ENTRY}\" ")), normalized);
        assert_eq!(normalize(&ENTRY.replace('\\', "/")), normalized);
        assert_eq!(normalize("C:\\"), "c:\\");
        assert_eq!(normalize("C:"), "c:\\");
        // Expansion is exercised through a variable the process already has:
        // setting one here would race every other test thread reading the
        // environment, which is why `set_var` is unsafe in this edition.
        let drive = std::env::var("SystemDrive").unwrap();
        assert_eq!(
            normalize("%SystemDrive%\\Windows"),
            normalize(&format!("{drive}\\Windows"))
        );
        assert_eq!(normalize("%NOPE_UNSET%\\a"), "%nope_unset%\\a");
    }

    #[test]
    fn a_pre_existing_lookalike_is_neither_duplicated_nor_claimed_nor_removed() {
        let lookalike = format!("{ENTRY}\\");
        let text = format!("C:\\Windows;{lookalike}");
        assert!(contains_equivalent(&text, &normalize(ENTRY)));
        // The planner keeps the entry as pre-existing, so nothing is
        // appended; and a removal keyed on our own raw text finds the
        // lookalike only through the normalized fallback, which the planner
        // never asks for when the entry pre-existed.
        assert_eq!(find_owned(&text, ENTRY, &normalize(ENTRY)), Some(1));
        assert_eq!(find_owned(&text, &lookalike, &normalize(ENTRY)), Some(1));
    }

    #[test]
    fn removal_keeps_a_trailing_empty_segment_and_prefers_the_raw_text() {
        let upper = ENTRY.to_uppercase();
        let text = format!("{ENTRY}\\;{upper};;");
        assert_eq!(
            split(&text),
            vec![format!("{ENTRY}\\"), upper.clone(), "".into(), "".into()]
        );
        assert_eq!(
            removed(&text, &upper, &normalize(ENTRY)).unwrap(),
            format!("{ENTRY}\\;;"),
            "the exact raw text goes, the lookalike and the empty segments stay"
        );
        assert_eq!(
            removed(&text, "C:\\elsewhere", &normalize(ENTRY)).unwrap(),
            format!("{upper};;"),
            "without an exact match the first normalized match goes"
        );
        assert_eq!(removed("C:\\Windows;", ENTRY, &normalize(ENTRY)), None);
        assert_eq!(removed("", ENTRY, &normalize(ENTRY)), None);
        assert_eq!(removed(ENTRY, ENTRY, &normalize(ENTRY)).unwrap(), "");
    }

    #[test]
    fn appending_keeps_a_trailing_empty_segment_and_round_trips() {
        assert_eq!(appended("", ENTRY), ENTRY);
        assert_eq!(
            appended("C:\\Windows", ENTRY),
            format!("C:\\Windows;{ENTRY}")
        );
        let with_empty = "C:\\Windows;C:\\Tools;;";
        let after = appended(with_empty, ENTRY);
        assert_eq!(after, format!("C:\\Windows;C:\\Tools;;{ENTRY};"));
        assert_eq!(
            removed(&after, ENTRY, &normalize(ENTRY)).unwrap(),
            with_empty,
            "removal restores the original text exactly"
        );
    }

    #[test]
    fn exactly_one_entry_after_a_reinstall_that_keeps_it() {
        let once = appended("C:\\Windows", ENTRY);
        assert!(contains_equivalent(&once, &normalize(ENTRY)));
        // A second install sees the equivalent and appends nothing.
        let count = split(&once)
            .iter()
            .filter(|s| normalize(s) == normalize(ENTRY))
            .count();
        assert_eq!(count, 1);
    }

    #[test]
    fn add_and_remove_are_idempotent_against_the_registry() {
        let prefix = format!(
            "Software\\TigerSetupTests\\path-{}-{}",
            std::process::id(),
            crate::report::unique_id()
        );
        let roots = Roots::relocated(&prefix);
        let key = KeyPath::parse("HKCU\\Environment").unwrap();
        assert_eq!(
            read(&roots, &key).unwrap(),
            (String::new(), false),
            "a scope with no environment key has no Path"
        );
        assert!(add(&roots, &key, ENTRY).unwrap());
        assert!(!add(&roots, &key, &format!("{ENTRY}\\")).unwrap());
        assert_eq!(read(&roots, &key).unwrap(), (ENTRY.to_string(), true));
        assert_eq!(
            registry::read_value(&roots, &key, VALUE_NAME).unwrap(),
            Some(Data::ExpandString(ENTRY.into())),
            "written as REG_EXPAND_SZ"
        );
        assert!(remove(&roots, &key, ENTRY, true).unwrap());
        assert!(!remove(&roots, &key, ENTRY, true).unwrap());
        assert_eq!(
            read(&roots, &key).unwrap(),
            (String::new(), false),
            "a Path TigerSetup created is deleted when it empties"
        );
        registry::delete_key_if_empty(&roots, &key).unwrap();
        let mut cleanup = KeyPath::new(registry::Hive::CurrentUser, prefix);
        while !cleanup.subkey.eq_ignore_ascii_case("software") {
            let _ = registry::delete_key_if_empty(&Roots::real(), &cleanup);
            match cleanup.parent() {
                Some(parent) => cleanup = parent,
                None => break,
            }
        }
    }
}
