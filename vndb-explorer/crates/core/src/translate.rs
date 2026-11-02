//! Simple machine translation for descriptions and tag names, with a local cache.
//!
//! Uses the free Google Translate web endpoint and falls back to MyMemory when Google refuses
//! (rate limits, captcha). Every translated string is cached in `user.sqlite`, so a text is
//! translated once and then works offline.

use anyhow::{bail, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::time::Duration;

fn cached(c: &Connection, src: &str, to: &str) -> Option<String> {
    c.query_row(
        "SELECT dst FROM translations WHERE src = ?1 AND lang = ?2",
        params![src, to],
        |r| r.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

fn store(c: &Connection, src: &str, to: &str, dst: &str) {
    let _ = c.execute(
        "INSERT OR REPLACE INTO translations(src, lang, dst) VALUES (?1, ?2, ?3)",
        params![src, to, dst],
    );
}

/// Split long text into pieces of at most `max` chars on paragraph / sentence boundaries.
fn chunks(text: &str, max: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for piece in text.split_inclusive(['\n', '.', '!', '?']) {
        if cur.chars().count() + piece.chars().count() > max && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if piece.chars().count() > max {
            // very long sentence: hard split on spaces
            for word in piece.split_inclusive(' ') {
                if cur.chars().count() + word.chars().count() > max && !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                cur.push_str(word);
            }
        } else {
            cur.push_str(piece);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// MyMemory returns some characters as HTML entities (`&#10;`, `&quot;` ...).
fn unescape_html(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        let Some(end) = tail.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let ent = &tail[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ if ent.starts_with("#x") => u32::from_str_radix(&ent[2..], 16)
                .ok()
                .and_then(char::from_u32),
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &tail[end + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Clean up a translated tag name; None if it does not look like a real translation
/// (MyMemory sometimes returns transliterations or numbered fragments from its memory).
fn tidy_line(src: &str, dst: &str, to: &str) -> Option<String> {
    let mut d = dst.trim().to_string();
    if !src.trim_start().starts_with(|c: char| c.is_ascii_digit()) {
        let digits = d.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits > 0 && d[digits..].starts_with('.') {
            d = d[digits + 1..].trim_start().to_string();
        }
    }
    if !src.trim_end().ends_with('.') {
        d = d.trim_end_matches('.').to_string();
    }
    let cyrillic = d.chars().any(|c| ('\u{400}'..='\u{4ff}').contains(&c));
    if (to == "ru" || to == "uk") && !cyrillic {
        return None;
    }
    (!d.is_empty()).then_some(d)
}

struct Engines {
    http: reqwest::blocking::Client,
    google_ok: bool,
}

impl Engines {
    fn google(&mut self, text: &str, to: &str) -> Result<String> {
        if !self.google_ok {
            bail!("google disabled");
        }
        let r = self
            .http
            .get("https://translate.googleapis.com/translate_a/single")
            .query(&[
                ("client", "gtx"),
                ("sl", "auto"),
                ("tl", to),
                ("dt", "t"),
                ("q", text),
            ])
            .send();
        let parsed = r
            .ok()
            .filter(|r| r.status().is_success())
            .and_then(|r| r.json::<Value>().ok());
        let Some(v) = parsed else {
            self.google_ok = false;
            bail!("google translate is not available");
        };
        let mut out = String::new();
        for seg in v[0].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(s) = seg[0].as_str() {
                out.push_str(s);
            }
        }
        if out.is_empty() {
            bail!("empty translation");
        }
        Ok(out)
    }

    fn mymemory(&mut self, text: &str, to: &str) -> Result<String> {
        let mut out = String::new();
        for piece in chunks(text, 450) {
            let v: Value = self
                .http
                .get("https://api.mymemory.translated.net/get")
                .query(&[("q", piece.as_str()), ("langpair", &format!("en|{to}"))])
                .send()?
                .json()?;
            if v["responseStatus"].as_i64().unwrap_or(0) != 200 {
                bail!(
                    "{}",
                    v["responseDetails"].as_str().unwrap_or("MyMemory error")
                );
            }
            let t = unescape_html(
                v["responseData"]["translatedText"]
                    .as_str()
                    .unwrap_or(&piece),
            );
            out.push_str(t.trim_start_matches(' '));
            if piece.ends_with('\n') && !out.ends_with('\n') {
                out.push('\n');
            }
        }
        Ok(out)
    }

    /// Long text: Google in chunks of ≤1500 chars, otherwise MyMemory.
    fn text(&mut self, text: &str, to: &str) -> Result<String> {
        let mut out = String::new();
        let mut google = true;
        for piece in chunks(text, 1500) {
            match google.then(|| self.google(&piece, to)) {
                Some(Ok(t)) => out.push_str(&t),
                _ => {
                    google = false;
                    out.push_str(&self.mymemory(&piece, to)?);
                }
            }
        }
        Ok(out)
    }
}

/// Translate `texts` into `to` ("ru", "en", ...). Returns translations in the same order.
pub fn translate(cache: &Connection, texts: &[String], to: &str) -> Result<Vec<String>> {
    let to: String = to
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || *c == '-')
        .take(8)
        .collect();
    let mut out: Vec<Option<String>> = texts.iter().map(|t| cached(cache, t, &to)).collect();
    let missing: Vec<usize> = (0..texts.len())
        .filter(|i| out[*i].is_none() && !texts[*i].trim().is_empty())
        .collect();
    if !missing.is_empty() {
        let mut eng = Engines {
            http: crate::http::blocking_client(Some(Duration::from_secs(20)))?,
            google_ok: true,
        };
        // Short single-line strings (tag names) go in one request, one per line.
        let short: Vec<usize> = missing
            .iter()
            .copied()
            .filter(|i| texts[*i].len() < 120 && !texts[*i].contains('\n'))
            .collect();
        // Groups of lines small enough for one request (MyMemory allows ~500 chars).
        let mut groups: Vec<Vec<usize>> = Vec::new();
        let mut len = 0;
        for i in &short {
            if groups.is_empty() || len + texts[*i].len() + 1 > 440 {
                groups.push(Vec::new());
                len = 0;
            }
            len += texts[*i].len() + 1;
            groups.last_mut().unwrap().push(*i);
        }
        for g in groups.iter().filter(|g| g.len() > 1) {
            let joined = g
                .iter()
                .map(|i| texts[*i].as_str())
                .collect::<Vec<_>>()
                .join("\n");
            let t = eng
                .google(&joined, &to)
                .or_else(|_| eng.mymemory(&joined, &to));
            if let Ok(t) = t {
                let lines: Vec<&str> = t.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
                if lines.len() == g.len() {
                    for (i, l) in g.iter().zip(lines) {
                        match tidy_line(&texts[*i], l, &to) {
                            Some(l) => {
                                store(cache, &texts[*i], &to, &l);
                                out[*i] = Some(l);
                            }
                            // keep the original name rather than a broken translation
                            None => out[*i] = Some(texts[*i].clone()),
                        }
                    }
                }
            }
        }
        let mut last_err = None;
        for i in missing {
            if out[i].is_some() {
                continue;
            }
            match eng.text(&texts[i], &to) {
                Ok(t) => {
                    store(cache, &texts[i], &to, &t);
                    out[i] = Some(t);
                }
                Err(e) => last_err = Some(e),
            }
        }
        if out
            .iter()
            .zip(texts)
            .any(|(o, t)| o.is_none() && !t.trim().is_empty())
        {
            if let Some(e) = last_err {
                if out.iter().all(|o| o.is_none()) {
                    return Err(e.context("translation service is not available"));
                }
            }
        }
    }
    Ok(out
        .into_iter()
        .zip(texts)
        .map(|(o, t)| o.unwrap_or_else(|| t.clone()))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn chunking_keeps_text() {
        let t = "One sentence. Two sentence!\nThree? ".repeat(40);
        let c = chunks(&t, 100);
        assert!(c.iter().all(|x| x.chars().count() <= 100));
        assert_eq!(c.concat(), t);
    }

    #[test]
    fn tidy_tag_translations() {
        assert_eq!(
            tidy_line("Health Issues", "17. Проблемы здоровья.", "ru").as_deref(),
            Some("Проблемы здоровья")
        );
        assert_eq!(
            tidy_line("Identity Crisis", "IDENTICHNOSTI KRIZIS", "ru"),
            None
        );
        assert_eq!(
            tidy_line("Time Travel", "Путешествия во времени", "ru").as_deref(),
            Some("Путешествия во времени")
        );
    }

    #[test]
    fn html_entities() {
        assert_eq!(
            unescape_html("a&#10; b &quot;c&quot; &amp; d &x"),
            "a\n b \"c\" & d &x"
        );
    }
}
