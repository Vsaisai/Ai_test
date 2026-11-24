//! Small, allocation-light helpers for reading XenForo pages (F95Zone).
//!
//! The site is parsed without a full HTML library: every function here works on byte
//! slices and never cuts through a multi-byte character, so a broken or unusual page can
//! only produce wrong text, never a panic (see the `multibyte_text_never_breaks_parsing`
//! test in the parent module).

/// Unescape the entities the site uses (`&amp;`, `&#39;`, `&#x27;`, …). Unknown sequences
/// are kept as-is. Never splits a multi-byte character: the search window for `;` is
/// shrunk to the previous char boundary first.
pub fn unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // look for ';' among the first 12 bytes, never cutting a multi-byte character
        let mut win = rest.len().min(12);
        while !rest.is_char_boundary(win) {
            win -= 1;
        }
        let end = rest[..win].find(';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Visible text of an HTML fragment, one line per block.
pub fn text(html: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        // look for ';' among the first 12 bytes, never cutting a multi-byte character
        let mut win = rest.len().min(12);
        while !rest.is_char_boundary(win) {
            win -= 1;
        }
        let end = rest[..win].find(';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Visible text of an HTML fragment, one line per block.
pub fn text(html: &str) -> String {
    // a line break in the page source is just a space; only tags break lines
    fn push(out: &mut String, text: &str) {
        out.extend(text.chars().map(|c| {
            if matches!(c, '\n' | '\r' | '\t') {
                ' '
            } else {
                c
            }
        }));
    }
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        push(&mut out, &rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[i + 1..i + j].trim_start_matches('/').to_lowercase();
        let name = tag.split([' ', '/']).next().unwrap_or("");
        if matches!(
            name,
            "br" | "div" | "p" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "blockquote"
        ) {
            out.push('\n');
        }
        // drop script / style contents
        if !rest[i + 1..].starts_with('/') && (name == "script" || name == "style") {
            let close = format!("</{name}");
            // ASCII lower case keeps byte positions (full lower case can change them)
            rest = match rest[i..].to_ascii_lowercase().find(&close) {
                Some(k) => &rest[i + k..],
                None => "",
            };
            continue;
        }
        rest = &rest[i + j + 1..];
    }
    push(&mut out, rest);
    let unescaped = unescape(&out);
    let mut lines: Vec<String> = Vec::new();
    for l in unescaped.lines() {
        let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() && lines.last().is_none_or(|x| x.is_empty()) {
            continue;
        }
        lines.push(l);
    }
    lines.join("\n").trim().to_string()
}

}

/// Text between the first `start` and the next `end` after it.
pub(super) fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)?;
    Some(&s[i..i + j])
}

/// All values of an attribute like `href="..."`, with the byte position of each match.
pub(super) fn attr_values<'a>(html: &'a str, attr: &str) -> Vec<(usize, &'a str)> {
    let pat = format!("{attr}=\"");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find(&pat) {
        let s = from + i + pat.len();
        let Some(e) = html[s..].find('"') else { break };
        out.push((from + i, &html[s..s + e]));
        from = s + e;
    }
    out
}

/// Prefix labels inside an HTML fragment: `<span class="label ...">Completed</span>`.
pub(super) fn labels(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<span class=\"label") {
        let Some(gt) = rest[i..].find('>') else { break };
        let s = i + gt + 1;
        let Some(e) = rest[s..].find("</span>") else {
            break;
        };
        out.push(text(&rest[s..s + e]));
        rest = &rest[s + e..];
    }
    out
}
