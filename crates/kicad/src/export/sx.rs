//! S-expression writer for KiCad files.
//!
//! A tree of [`X`] built with small helpers, written the way KiCad writes its files: a
//! list's atoms on its first line, its lists on lines of their own one tab further in,
//! short lists of atoms (`(at 1 2 90)`) on one line.

use model::Coord;

#[derive(Clone, Debug, PartialEq)]
pub enum X {
    /// A bare word or number.
    Sym(String),
    /// A quoted string.
    Str(String),
    List(Vec<X>),
}

/// A list `(head …)`.
pub fn l(head: &str) -> X {
    X::List(vec![X::Sym(head.to_string())])
}

/// mm of a length in nm, exactly (up to 6 decimals, trailing zeros dropped).
pub fn mm(v: Coord) -> String {
    let sign = if v < 0 { "-" } else { "" };
    let a = v.unsigned_abs();
    let (whole, frac) = (a / 1_000_000, a % 1_000_000);
    if frac == 0 {
        return format!("{sign}{whole}");
    }
    let f = format!("{frac:06}");
    format!("{sign}{whole}.{}", f.trim_end_matches('0'))
}

/// A number with up to 6 decimals, without a trailing `.0` or `-0`.
pub fn num(v: f64) -> String {
    let r = (v * 1e6).round() / 1e6;
    let s = format!("{r}");
    if s == "-0" { "0".into() } else { s }
}

impl X {
    /// Append a bare word.
    pub fn sym(self, s: impl Into<String>) -> X {
        self.push(X::Sym(s.into()))
    }

    /// Append a quoted string.
    pub fn str(self, s: impl Into<String>) -> X {
        self.push(X::Str(s.into()))
    }

    /// Append a length in mm.
    pub fn mm(self, v: Coord) -> X {
        self.push(X::Sym(mm(v)))
    }

    /// Append a number.
    pub fn num(self, v: f64) -> X {
        self.push(X::Sym(num(v)))
    }

    /// Append `(head yes|no)`.
    pub fn yes_no(self, head: &str, v: bool) -> X {
        self.push(l(head).sym(if v { "yes" } else { "no" }))
    }

    pub fn push(mut self, x: X) -> X {
        self.add(x);
        self
    }

    pub fn add(&mut self, x: X) {
        if let X::List(items) = self {
            items.push(x);
        }
    }

    pub fn extend(mut self, xs: impl IntoIterator<Item = X>) -> X {
        for x in xs {
            self.add(x);
        }
        self
    }

    /// Append `x` if there is one.
    pub fn opt(self, x: Option<X>) -> X {
        match x {
            Some(x) => self.push(x),
            None => self,
        }
    }

    /// The file text, with a final newline.
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        self.write(&mut out, 0);
        out.push('\n');
        out
    }

    fn is_list(&self) -> bool {
        matches!(self, X::List(_))
    }

    /// Written on one line: no lists inside.
    fn inline(&self) -> bool {
        match self {
            X::List(items) => !items.iter().any(X::is_list),
            _ => true,
        }
    }

    fn write(&self, out: &mut String, depth: usize) {
        match self {
            X::Sym(s) => out.push_str(s),
            X::Str(s) => quote(s, out),
            X::List(items) => {
                out.push('(');
                let inline = self.inline() || is_points(self);
                let mut first = true;
                for item in items {
                    if item.is_list() && !inline {
                        out.push('\n');
                        for _ in 0..=depth {
                            out.push('\t');
                        }
                    } else if !first {
                        out.push(' ');
                    }
                    item.write(out, depth + 1);
                    first = false;
                }
                if !inline && items.iter().any(X::is_list) {
                    out.push('\n');
                    for _ in 0..depth {
                        out.push('\t');
                    }
                }
                out.push(')');
            }
        }
    }
}

/// `(pts (xy …) …)`: the points on one line.
fn is_points(x: &X) -> bool {
    match x {
        X::List(items) => {
            items.first() == Some(&X::Sym("pts".into())) && items.iter().skip(1).all(|i| i.inline() && i.is_list())
        }
        _ => false,
    }
}

fn quote(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sexpr;

    #[test]
    fn numbers_are_exact() {
        assert_eq!(mm(1_270_000), "1.27");
        assert_eq!(mm(-500_000), "-0.5");
        assert_eq!(mm(0), "0");
        assert_eq!(mm(1), "0.000001");
        assert_eq!(mm(-25_400_000), "-25.4");
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(90.0), "90");
        assert_eq!(num(0.1 + 0.2), "0.3");
    }

    #[test]
    fn written_text_parses_back() {
        let x = l("kicad_sch")
            .push(l("version").sym("20250114"))
            .push(l("text").str("a \"b\"\\c\nd").push(l("at").mm(1_000_000).mm(-2_540_000).num(90.0)))
            .push(l("polyline").push(l("pts").push(l("xy").mm(0).mm(0)).push(l("xy").mm(1).mm(2))));
        let text = x.to_text();
        assert!(text.contains("\n\t(version 20250114)"), "{text}");
        assert!(text.contains("(pts (xy 0 0) (xy 0.000001 0.000002))"), "{text}");
        let n = sexpr::parse(&text).unwrap();
        let t = n.child("text").unwrap();
        assert_eq!(t.arg(0), Some("a \"b\"\\c\nd"));
        assert_eq!(t.child("at").unwrap().num(2), Some(90.0));
    }
}
