//! Minimal S-expression reader for KiCad files.
//!
//! KiCad files are lists of the form `(head arg arg (child ...) ...)`. Atoms are bare words
//! or numbers, or quoted strings with backslash escapes. Flags appear both as bare atoms
//! (`hide`, KiCad 6/7) and as lists (`(hide yes)`, KiCad 8+); [`Node::flag`] handles both.

use std::fmt;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    List(Vec<Node>),
    Atom(String),
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("line {line}: {message}")]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

pub fn parse(text: &str) -> Result<Node, ParseError> {
    let mut p = Parser { s: text.as_bytes(), i: 0, line: 1 };
    p.skip_ws();
    let node = p.node()?;
    p.skip_ws();
    if p.i < p.s.len() {
        return Err(p.err("unexpected data after the top-level list"));
    }
    Ok(node)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    line: usize,
}

impl Parser<'_> {
    fn err(&self, m: &str) -> ParseError {
        ParseError { line: self.line, message: m.to_string() }
    }

    fn skip_ws(&mut self) {
        while let Some(&c) = self.s.get(self.i) {
            if c == b'\n' {
                self.line += 1;
            }
            if !c.is_ascii_whitespace() {
                break;
            }
            self.i += 1;
        }
    }

    fn node(&mut self) -> Result<Node, ParseError> {
        match self.s.get(self.i) {
            None => Err(self.err("unexpected end of file")),
            Some(b'(') => {
                self.i += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    match self.s.get(self.i) {
                        None => return Err(self.err("unclosed list")),
                        Some(b')') => {
                            self.i += 1;
                            return Ok(Node::List(items));
                        }
                        Some(_) => items.push(self.node()?),
                    }
                }
            }
            Some(b')') => Err(self.err("unexpected ')'")),
            Some(b'"') => self.string(),
            Some(_) => {
                let start = self.i;
                while let Some(&c) = self.s.get(self.i) {
                    if c.is_ascii_whitespace() || c == b'(' || c == b')' {
                        break;
                    }
                    self.i += 1;
                }
                Ok(Node::Atom(String::from_utf8_lossy(&self.s[start..self.i]).into_owned()))
            }
        }
    }

    fn string(&mut self) -> Result<Node, ParseError> {
        self.i += 1; // opening quote
        let mut out = Vec::new();
        loop {
            match self.s.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(Node::Atom(String::from_utf8_lossy(&out).into_owned()));
                }
                Some(b'\\') => {
                    let c = *self.s.get(self.i + 1).ok_or_else(|| self.err("unterminated string"))?;
                    out.push(match c {
                        b'n' => b'\n',
                        b't' => b'\t',
                        b'r' => b'\r',
                        other => other,
                    });
                    self.i += 2;
                }
                Some(&c) => {
                    if c == b'\n' {
                        self.line += 1;
                    }
                    out.push(c);
                    self.i += 1;
                }
            }
        }
    }
}

impl Node {
    pub fn as_atom(&self) -> Option<&str> {
        match self {
            Node::Atom(s) => Some(s),
            Node::List(_) => None,
        }
    }

    pub fn items(&self) -> &[Node] {
        match self {
            Node::List(v) => v,
            Node::Atom(_) => &[],
        }
    }

    /// First atom of a list (`footprint`, `pad`, `at`, …).
    pub fn head(&self) -> &str {
        self.items().first().and_then(Node::as_atom).unwrap_or("")
    }

    pub fn is(&self, head: &str) -> bool {
        matches!(self, Node::List(_)) && self.head() == head
    }

    /// The `i`-th argument after the head, if it is an atom.
    pub fn arg(&self, i: usize) -> Option<&str> {
        self.items().get(i + 1).and_then(Node::as_atom)
    }

    pub fn num(&self, i: usize) -> Option<f64> {
        self.arg(i).and_then(|s| s.parse().ok())
    }

    /// Child lists (`(head …)` items after the head).
    pub fn lists(&self) -> impl Iterator<Item = &Node> {
        self.items().iter().skip(1).filter(|n| matches!(n, Node::List(_)))
    }

    pub fn child(&self, head: &str) -> Option<&Node> {
        self.lists().find(|n| n.head() == head)
    }

    pub fn children<'a>(&'a self, head: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.lists().filter(move |n| n.head() == head)
    }

    /// A flag given as a bare atom (`hide`) or as `(hide)` / `(hide yes)`.
    pub fn flag(&self, name: &str) -> bool {
        self.items().iter().skip(1).any(|n| match n {
            Node::Atom(a) => a == name,
            Node::List(_) => n.head() == name && n.arg(0).is_none_or(|v| v == "yes" || v == "true"),
        })
    }

    /// `(head x y …)` of a child as a pair of numbers.
    pub fn xy(&self, head: &str) -> Option<(f64, f64)> {
        let c = self.child(head)?;
        Some((c.num(0)?, c.num(1)?))
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Node::Atom(a) => write!(f, "{a:?}"),
            Node::List(items) => {
                f.write_str("(")?;
                for (i, n) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(" ")?;
                    }
                    write!(f, "{n}")?;
                }
                f.write_str(")")
            }
        }
    }
}

/// Names of the top-level `(symbol "NAME" …)` entries of a symbol library, without
/// building the tree (fast enough to index all of KiCad's libraries at startup).
pub fn top_level_names(text: &str, head: &str) -> Vec<String> {
    let s = text.as_bytes();
    let mut names = Vec::new();
    let mut depth = 0usize;
    let mut i = 0;
    while i < s.len() {
        match s[i] {
            b'"' => {
                i += 1;
                while i < s.len() && s[i] != b'"' {
                    i += if s[i] == b'\\' { 2 } else { 1 };
                }
            }
            b'(' => {
                depth += 1;
                if depth == 2 && s[i + 1..].starts_with(head.as_bytes()) {
                    let rest = &text[i + 1 + head.len()..];
                    if rest.starts_with(|c: char| c.is_ascii_whitespace())
                        && let Ok(Node::Atom(name)) = (Parser { s: rest.trim_start().as_bytes(), i: 0, line: 0 }).node()
                    {
                        names.push(name);
                    }
                }
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_atoms_strings_and_flags() {
        let n =
            parse("(pin input line (at -7.62 2.54 0) hide (name \"A \\\"q\\\"\" (effects)) (number \"3\"))").unwrap();
        assert_eq!(n.head(), "pin");
        assert_eq!(n.arg(0), Some("input"));
        assert_eq!(n.xy("at"), Some((-7.62, 2.54)));
        assert_eq!(n.child("at").unwrap().num(2), Some(0.0));
        assert!(n.flag("hide"));
        assert_eq!(n.child("name").unwrap().arg(0), Some("A \"q\""));
        let m = parse("(pin (hide yes) (x (hide no)))").unwrap();
        assert!(m.flag("hide"));
        assert!(!m.child("x").unwrap().flag("hide"));
    }

    #[test]
    fn errors_have_line_numbers() {
        let e = parse("(a\n (b \"x)\n").unwrap_err();
        assert_eq!(e.line, 3);
        assert!(parse("(a))").is_err());
    }

    #[test]
    fn names_without_parsing() {
        let text = "(kicad_symbol_lib (version 1)\n (symbol \"R\" (symbol \"R_0_1\"))\n (symbol \"C (x)\" (property \"(symbol \\\"no\\\"\")))";
        assert_eq!(top_level_names(text, "symbol"), ["R", "C (x)"]);
    }
}
