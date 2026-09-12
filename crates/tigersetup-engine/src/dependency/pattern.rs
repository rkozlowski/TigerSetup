//! The regular-expression subset detector patterns use, matched against a
//! whole string without regard to letter case: literals, `.`, character
//! classes (`[0-9]`, `[^a-z]`), the escapes `\d`, `\w`, `\s` and `\<char>`,
//! the quantifiers `*`, `+`, `?` and `{m}`, `{m,}`, `{m,n}`, groups and
//! alternation. A pattern matches a name only as a whole, so `10\..*`
//! matches `10.0.11` and not `110.0.1`; `^` and `$` are accepted at the
//! ends and mean nothing more than the whole-string rule already does.

use crate::{Error, Result};

#[derive(Debug, Clone, PartialEq, Eq)]
enum Atom {
    Any,
    Char(char),
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
    Group(Vec<Vec<Piece>>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Piece {
    atom: Atom,
    min: u32,
    max: Option<u32>,
}

/// A compiled pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pattern {
    alternatives: Vec<Vec<Piece>>,
    source: String,
}

fn fold(c: char) -> char {
    c.to_lowercase().next().unwrap_or(c)
}

struct Parser<'a> {
    chars: Vec<char>,
    at: usize,
    source: &'a str,
}

impl Parser<'_> {
    fn invalid(&self, why: &str) -> Error {
        Error::new(
            "metadata_invalid",
            format!("pattern {:?}: {why}", self.source),
        )
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn next(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += 1;
        Some(c)
    }

    fn alternatives(&mut self, depth: u32) -> Result<Vec<Vec<Piece>>> {
        let mut alternatives = vec![self.sequence(depth)?];
        while self.peek() == Some('|') {
            self.at += 1;
            alternatives.push(self.sequence(depth)?);
        }
        Ok(alternatives)
    }

    fn sequence(&mut self, depth: u32) -> Result<Vec<Piece>> {
        let mut pieces = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || (c == ')' && depth > 0) {
                break;
            }
            if c == '^' && self.at == 0 {
                self.at += 1;
                continue;
            }
            if c == '$' && self.at + 1 == self.chars.len() {
                self.at += 1;
                continue;
            }
            let atom = self.atom(depth)?;
            let (min, max) = self.quantifier()?;
            pieces.push(Piece { atom, min, max });
        }
        Ok(pieces)
    }

    fn atom(&mut self, depth: u32) -> Result<Atom> {
        let c = self.next().ok_or_else(|| self.invalid("unexpected end"))?;
        Ok(match c {
            '(' => {
                if self.peek() == Some('?') {
                    self.at += 1;
                    if self.next() != Some(':') {
                        return Err(self.invalid("only (?: groups are supported"));
                    }
                }
                let inner = self.alternatives(depth + 1)?;
                if self.next() != Some(')') {
                    return Err(self.invalid("unclosed group"));
                }
                Atom::Group(inner)
            }
            ')' => return Err(self.invalid("unmatched )")),
            '[' => self.class()?,
            '.' => Atom::Any,
            '\\' => self.escape()?,
            '*' | '+' | '?' => return Err(self.invalid("quantifier without an atom")),
            c => Atom::Char(fold(c)),
        })
    }

    fn escape(&mut self) -> Result<Atom> {
        let c = self
            .next()
            .ok_or_else(|| self.invalid("dangling backslash"))?;
        Ok(match c {
            'd' => Atom::Class {
                negated: false,
                ranges: vec![('0', '9')],
            },
            'w' => Atom::Class {
                negated: false,
                ranges: vec![('a', 'z'), ('0', '9'), ('_', '_')],
            },
            's' => Atom::Class {
                negated: false,
                ranges: vec![(' ', ' '), ('\t', '\t'), ('\n', '\n'), ('\r', '\r')],
            },
            c => Atom::Char(fold(c)),
        })
    }

    fn class(&mut self) -> Result<Atom> {
        let negated = self.peek() == Some('^');
        if negated {
            self.at += 1;
        }
        let mut ranges = Vec::new();
        let mut first = true;
        loop {
            let c = self.next().ok_or_else(|| self.invalid("unclosed ["))?;
            if c == ']' && !first {
                break;
            }
            first = false;
            let low = if c == '\\' {
                self.next()
                    .ok_or_else(|| self.invalid("dangling backslash"))?
            } else {
                c
            };
            let high = if self.peek() == Some('-') && self.chars.get(self.at + 1) != Some(&']') {
                self.at += 1;
                let h = self.next().ok_or_else(|| self.invalid("unclosed ["))?;
                if h == '\\' {
                    self.next()
                        .ok_or_else(|| self.invalid("dangling backslash"))?
                } else {
                    h
                }
            } else {
                low
            };
            ranges.push((fold(low), fold(high)));
        }
        Ok(Atom::Class { negated, ranges })
    }

    fn number(&mut self) -> Option<u32> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        if start == self.at {
            return None;
        }
        self.chars[start..self.at]
            .iter()
            .collect::<String>()
            .parse()
            .ok()
    }

    fn quantifier(&mut self) -> Result<(u32, Option<u32>)> {
        Ok(match self.peek() {
            Some('*') => {
                self.at += 1;
                (0, None)
            }
            Some('+') => {
                self.at += 1;
                (1, None)
            }
            Some('?') => {
                self.at += 1;
                (0, Some(1))
            }
            Some('{') => {
                self.at += 1;
                let min = self.number().ok_or_else(|| self.invalid("bad {m,n}"))?;
                let max = if self.peek() == Some(',') {
                    self.at += 1;
                    self.number()
                } else {
                    Some(min)
                };
                if self.next() != Some('}') {
                    return Err(self.invalid("bad {m,n}"));
                }
                if max.is_some_and(|max| max < min) {
                    return Err(self.invalid("bad {m,n}"));
                }
                (min, max)
            }
            _ => (1, Some(1)),
        })
    }
}

impl Pattern {
    pub fn compile(source: &str) -> Result<Pattern> {
        let mut parser = Parser {
            chars: source.chars().collect(),
            at: 0,
            source,
        };
        let alternatives = parser.alternatives(0)?;
        if parser.at != parser.chars.len() {
            return Err(parser.invalid("unmatched )"));
        }
        Ok(Pattern {
            alternatives,
            source: source.to_string(),
        })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Whether the whole of `text` matches.
    pub fn is_match(&self, text: &str) -> bool {
        let chars: Vec<char> = text.chars().map(fold).collect();
        match_alternatives(&self.alternatives, &chars, 0, &|end| end == chars.len())
    }
}

fn match_alternatives(
    alternatives: &[Vec<Piece>],
    text: &[char],
    at: usize,
    next: &dyn Fn(usize) -> bool,
) -> bool {
    alternatives
        .iter()
        .any(|sequence| match_sequence(sequence, 0, text, at, next))
}

fn match_sequence(
    sequence: &[Piece],
    index: usize,
    text: &[char],
    at: usize,
    next: &dyn Fn(usize) -> bool,
) -> bool {
    match sequence.get(index) {
        None => next(at),
        Some(piece) => match_repeat(piece, 0, sequence, index, text, at, next),
    }
}

#[allow(clippy::too_many_arguments)]
fn match_repeat(
    piece: &Piece,
    count: u32,
    sequence: &[Piece],
    index: usize,
    text: &[char],
    at: usize,
    next: &dyn Fn(usize) -> bool,
) -> bool {
    // Greedy: one more repetition first, as long as it consumes something.
    if piece.max.is_none_or(|max| count < max)
        && match_atom(&piece.atom, text, at, &|after| {
            after != at && match_repeat(piece, count + 1, sequence, index, text, after, next)
        })
    {
        return true;
    }
    count >= piece.min && match_sequence(sequence, index + 1, text, at, next)
}

fn match_atom(atom: &Atom, text: &[char], at: usize, next: &dyn Fn(usize) -> bool) -> bool {
    match atom {
        Atom::Any => at < text.len() && next(at + 1),
        Atom::Char(c) => at < text.len() && text[at] == *c && next(at + 1),
        Atom::Class { negated, ranges } => {
            at < text.len() && {
                let c = text[at];
                let inside = ranges.iter().any(|(low, high)| *low <= c && c <= *high);
                inside != *negated && next(at + 1)
            }
        }
        Atom::Group(alternatives) => match_alternatives(alternatives, text, at, next),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(pattern: &str, text: &str) -> bool {
        Pattern::compile(pattern).unwrap().is_match(text)
    }

    #[test]
    fn whole_string_case_insensitive_matching() {
        assert!(matches("10.*", "10.0.11"));
        assert!(matches("10.*", "10"));
        assert!(!matches("10\\..*", "10"));
        assert!(matches("10\\..*", "10.0.11"));
        assert!(!matches("10\\..*", "110.0.1"));
        assert!(!matches("10.*", "9.0.5"));
        assert!(matches(
            "microsoft edge webview2 runtime",
            "Microsoft Edge WebView2 Runtime"
        ));
        assert!(matches(
            "Microsoft Windows Desktop Runtime.*\\(x64\\)",
            "Microsoft Windows Desktop Runtime 10.0.11 (x64)"
        ));
        assert!(!matches(
            "Microsoft Windows Desktop Runtime.*\\(x64\\)",
            "Microsoft Windows Desktop Runtime 10.0.11 (x86)"
        ));
        assert!(matches("^\\d+(\\.\\d+){1,3}$", "10.0.4191.53"));
        assert!(!matches("^\\d+(\\.\\d+){1,3}$", "10.0.4191.53.1"));
        assert!(matches("[0-9]+", "2026"));
        assert!(!matches("[^0-9]+", "2026"));
        assert!(matches("(a|b)c?", "B"));
        assert!(matches("x{2}", "XX"));
        assert!(!matches("x{2}", "x"));
        assert!(matches("\\w+\\s\\w+", "hello world"));
        assert!(matches("(a*)*b", "aaab"), "empty repetitions do not loop");
        assert!(matches("", ""));
        assert!(!matches("", "x"));
    }

    #[test]
    fn invalid_patterns_are_refused() {
        for bad in ["(", ")", "[abc", "*a", "a{3,1}", "a{x}", "(?x)", "\\"] {
            assert_eq!(
                Pattern::compile(bad).unwrap_err().code,
                "metadata_invalid",
                "{bad}"
            );
        }
    }
}
