//! Parser for PostgreSQL `COPY` text format used by the VNDB dump (TSV with escapes).

/// Split one line into fields, unescaping each. `\N` becomes `None`.
pub fn split(line: &str) -> Vec<Option<String>> {
    line.split('\t').map(unescape).collect()
}

fn unescape(f: &str) -> Option<String> {
    if f == "\\N" {
        return None;
    }
    if !f.contains('\\') {
        return Some(f.to_string());
    }
    let mut out = String::with_capacity(f.len());
    let mut it = f.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('b') => out.push('\u{8}'),
            Some('f') => out.push('\u{c}'),
            Some('v') => out.push('\u{b}'),
            Some(o) => out.push(o),
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// Helpers to read typed columns.
pub struct Row(pub Vec<Option<String>>);

impl Row {
    pub fn s(&self, i: usize) -> Option<&str> {
        self.0.get(i).and_then(|v| v.as_deref())
    }
    pub fn string(&self, i: usize) -> String {
        self.s(i).unwrap_or("").to_string()
    }
    pub fn opt(&self, i: usize) -> Option<String> {
        self.s(i).filter(|s| !s.is_empty()).map(str::to_string)
    }
    pub fn b(&self, i: usize) -> bool {
        self.s(i) == Some("t")
    }
    pub fn ob(&self, i: usize) -> Option<bool> {
        self.s(i).map(|s| s == "t")
    }
    pub fn i(&self, i: usize) -> Option<i64> {
        self.s(i).and_then(|s| s.parse().ok())
    }
    pub fn f(&self, i: usize) -> Option<f64> {
        self.s(i).and_then(|s| s.parse().ok())
    }
    /// "v123" / "g5" / "r9" -> 123 / 5 / 9
    pub fn id(&self, i: usize) -> Option<u32> {
        self.s(i).and_then(vndbid)
    }
}

pub fn vndbid(s: &str) -> Option<u32> {
    s.trim_start_matches(|c: char| c.is_ascii_alphabetic())
        .parse()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_copy_lines() {
        let r = Row(split("v1\t\\N\tline\\nnext\ta\\\\b\tt"));
        assert_eq!(r.id(0), Some(1));
        assert_eq!(r.s(1), None);
        assert_eq!(r.string(2), "line\nnext");
        assert_eq!(r.string(3), "a\\b");
        assert!(r.b(4));
    }
}
