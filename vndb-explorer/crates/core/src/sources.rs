//! Third-party catalogs ("sources") such as vngames.ru: direct page matches + search links.
//!
//! A source may publish a sitemap whose page addresses are slugs of the title
//! (`https://vngames.ru/novels/detail/fatestay-night/`). The index of those slugs is stored
//! locally and matched against slugs generated from every VNDB title of a VN.
//! The list is configurable: `sources.json` in the data directory overrides the defaults.

use crate::catalog::Vn;
use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;
use unicode_normalization::UnicodeNormalization;

pub const DEFAULT: &str = include_str!("../assets/sources.json");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexDef {
    /// sitemap.xml (or sitemap index) URL
    pub sitemap: String,
    /// Page URLs starting with this prefix are entries; the rest of the path is the slug.
    pub prefix: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceDef {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub lang: String,
    /// "community" (always shown) or "store_search" (only when VNDB has no store / download links)
    #[serde(default = "community")]
    pub kind: String,
    #[serde(default)]
    pub adult: bool,
    #[serde(default)]
    pub description: Option<String>,
    /// Search URL with `{q}` placeholder.
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub index: Option<IndexDef>,
}

fn community() -> String {
    "community".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sources {
    pub sources: Vec<SourceDef>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SourceLink {
    pub source: String,
    pub name: String,
    pub lang: String,
    pub kind: String,
    pub adult: bool,
    pub url: String,
    /// true = the VN's own page on that site, false = a search link
    pub direct: bool,
}

impl Sources {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let custom = data_dir.join("sources.json");
        let text = if custom.exists() {
            std::fs::read_to_string(&custom)?
        } else {
            DEFAULT.to_string()
        };
        serde_json::from_str(&text)
            .with_context(|| format!("invalid sources file {}", custom.display()))
    }
}

/// Russian -> Latin the way vngames.ru builds its slugs.
fn translit(c: char) -> Option<&'static str> {
    Some(match c {
        'а' => "a",
        'б' => "b",
        'в' => "v",
        'г' => "g",
        'д' => "d",
        'е' => "e",
        'ё' => "yo",
        'ж' => "zh",
        'з' => "z",
        'и' => "i",
        'й' => "j",
        'к' => "k",
        'л' => "l",
        'м' => "m",
        'н' => "n",
        'о' => "o",
        'п' => "p",
        'р' => "r",
        'с' => "s",
        'т' => "t",
        'у' => "u",
        'ф' => "f",
        'х' => "kh",
        'ц' => "ts",
        'ч' => "ch",
        'ш' => "sh",
        'щ' => "shch",
        'ъ' => "",
        'ы' => "i",
        'ь' => "",
        'э' => "e",
        'ю' => "yu",
        'я' => "ya",
        'і' => "i",
        'ї' => "yi",
        'є' => "ye",
        'ґ' => "g",
        _ => return None,
    })
}

/// Django-style slug: lower-case ASCII letters/digits separated by single hyphens.
pub fn slugify(s: &str) -> String {
    let mut ascii = String::new();
    for c in s.to_lowercase().chars() {
        if let Some(t) = translit(c) {
            ascii.push_str(t);
            continue;
        }
        if c == '&' {
            ascii.push_str(" and ");
            continue;
        }
        ascii.extend(c.to_string().nfkd().filter(|d| d.is_ascii()));
    }
    let mut out = String::new();
    let mut dash = false;
    for c in ascii.chars() {
        if c.is_ascii_alphanumeric() {
            if dash && !out.is_empty() {
                out.push('-');
            }
            dash = false;
            out.push(c);
        } else if c == ' '
            || c == '-'
            || c == '_'
            || c == '\t'
            || c == '~'
            || c == ':'
            || c == '.'
            || c == ','
        {
            dash = true;
        }
        // other punctuation (/, ', !, ?, ...) is dropped without a separator, like Django
    }
    out
}

/// Every slug a VN could be filed under.
pub fn candidate_slugs(vn: &Vn, aliases: &[String]) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    let mut add = |s: &str| {
        let slug = slugify(s);
        if slug.len() >= 2 {
            set.insert(slug);
        }
    };
    for t in &vn.titles {
        add(&t.title);
        if let Some(l) = &t.latin {
            add(l);
        }
    }
    for a in aliases {
        add(a);
    }
    set
}

fn locs(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<loc>") {
        rest = &rest[i + 5..];
        let Some(j) = rest.find("</loc>") else { break };
        out.push(rest[..j].trim().replace("&amp;", "&"));
        rest = &rest[j + 6..];
    }
    out
}

/// Download sitemaps of all indexable sources and store their entries.
pub fn refresh_index(
    defs: &Sources,
    user: &Connection,
    progress: &dyn Fn(&str),
) -> Result<Vec<(String, usize)>> {
    let http = crate::http::blocking_client(Some(Duration::from_secs(60)))?;
    let mut result = Vec::new();
    for s in &defs.sources {
        let Some(ix) = &s.index else { continue };
        progress(&format!("Indexing {}", s.name));
        let mut queue = vec![ix.sitemap.clone()];
        let mut entries: Vec<(String, String)> = Vec::new();
        let mut fetched = 0;
        while let Some(url) = queue.pop() {
            fetched += 1;
            if fetched > 20 {
                break;
            }
            let text = match http
                .get(&url)
                .send()
                .and_then(|r| r.error_for_status())
                .and_then(|r| r.text())
            {
                Ok(t) => t,
                Err(e) => {
                    progress(&format!("{}: {e}", s.name));
                    continue;
                }
            };
            let is_index = text.contains("<sitemapindex");
            for loc in locs(&text) {
                if is_index {
                    queue.push(loc);
                } else if let Some(rest) = loc.strip_prefix(&ix.prefix) {
                    let key = rest.trim_end_matches('/').to_string();
                    if !key.is_empty() && !key.contains('/') {
                        entries.push((key, loc.clone()));
                    }
                }
            }
        }
        if entries.is_empty() {
            continue; // keep the previous index if the site was unreachable
        }
        let tx = user.unchecked_transaction()?;
        tx.execute("DELETE FROM source_index WHERE source = ?1", [&s.id])?;
        {
            let mut st = tx.prepare(
                "INSERT OR IGNORE INTO source_index(source, key, url) VALUES (?1,?2,?3)",
            )?;
            for (k, u) in &entries {
                st.execute(params![s.id, k, u])?;
            }
        }
        tx.execute(
            "INSERT OR REPLACE INTO source_meta(source, fetched_at, entries) VALUES (?1, datetime('now'), ?2)",
            params![s.id, entries.len()],
        )?;
        tx.commit()?;
        result.push((s.id.clone(), entries.len()));
    }
    Ok(result)
}

/// Days since the oldest index refresh (None = never indexed).
pub fn index_age_days(defs: &Sources, user: &Connection) -> Option<f64> {
    let mut worst: Option<f64> = None;
    for s in defs.sources.iter().filter(|s| s.index.is_some()) {
        let age: Option<f64> = user
            .query_row("SELECT julianday('now') - julianday(fetched_at) FROM source_meta WHERE source = ?1", [&s.id], |r| r.get(0))
            .ok();
        let a = age?;
        worst = Some(worst.map_or(a, |w: f64| w.max(a)));
    }
    worst.or(Some(0.0))
}

/// Match title slugs against a source index:
/// 1. exact slug;
/// 2. otherwise a page whose slug extends ours with a subtitle (`molytropia` -> `molytropia-cloud-in-shape-of-hurt`),
///    only if our slug is long enough and the page is unique;
/// 3. otherwise our slug extends the page slug (`shinseiki-evangelion-ikari-shinji-ikusei-keikaku` -> `ikari-shinji-ikusei-keikaku`).
pub fn match_index(user: &Connection, source: &str, slugs: &BTreeSet<String>) -> Vec<String> {
    let mut exact = Vec::new();
    {
        let Ok(mut st) =
            user.prepare_cached("SELECT url FROM source_index WHERE source = ?1 AND key = ?2")
        else {
            return vec![];
        };
        for slug in slugs {
            if let Ok(url) = st.query_row(params![source, slug], |r| r.get::<_, String>(0)) {
                if !exact.contains(&url) {
                    exact.push(url);
                }
            }
        }
    }
    if !exact.is_empty() {
        return exact;
    }
    let Ok(mut st) = user.prepare_cached(
        "SELECT key, url FROM source_index WHERE source = ?1 AND key >= ?2 AND key < ?3",
    ) else {
        return vec![];
    };
    for slug in slugs.iter().filter(|s| s.len() >= 8) {
        let lo = format!("{slug}-");
        let hi = format!("{slug}.");
        let found: Vec<String> = st
            .query_map(params![source, lo, hi], |r| r.get::<_, String>(1))
            .map(|rows| rows.filter_map(Result::ok).collect())
            .unwrap_or_default();
        if found.len() == 1 {
            return found;
        }
    }
    let Ok(mut st) = user.prepare_cached("SELECT key, url FROM source_index WHERE source = ?1")
    else {
        return vec![];
    };
    let keys: Vec<(String, String)> = st
        .query_map(params![source], |r| Ok((r.get(0)?, r.get(1)?)))
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let mut found = Vec::new();
    for (key, url) in keys
        .iter()
        .filter(|(k, _)| k.len() >= 12 && k.contains('-'))
    {
        if slugs.iter().any(|s| s.ends_with(&format!("-{key}"))) && !found.contains(url) {
            found.push(url.clone());
        }
    }
    if found.len() == 1 {
        found
    } else {
        vec![]
    }
}

/// Links to third-party sites for one VN.
pub fn links_for(
    defs: &Sources,
    user: &Connection,
    vn: &Vn,
    aliases: &[String],
    order: &[String],
    kind: &str,
) -> Vec<SourceLink> {
    let slugs = candidate_slugs(vn, aliases);
    let mut out = Vec::new();
    for s in defs.sources.iter().filter(|s| s.kind == kind) {
        let mut direct = false;
        if s.index.is_some() {
            for url in match_index(user, &s.id, &slugs) {
                out.push(SourceLink {
                    source: s.id.clone(),
                    name: s.name.clone(),
                    lang: s.lang.clone(),
                    kind: s.kind.clone(),
                    adult: s.adult,
                    url,
                    direct: true,
                });
                direct = true;
            }
        }
        if !direct {
            if let Some(tpl) = &s.search {
                let mut pref = vec![s.lang.clone()];
                pref.extend(order.iter().cloned());
                pref.push("en".into());
                let title = vn.display_title(&pref);
                out.push(SourceLink {
                    source: s.id.clone(),
                    name: s.name.clone(),
                    lang: s.lang.clone(),
                    kind: s.kind.clone(),
                    adult: s.adult,
                    url: tpl.replace("{q}", &urlencoding::encode(&title)),
                    direct: false,
                });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_like_vngames() {
        assert_eq!(slugify("Fate/stay night"), "fatestay-night");
        assert_eq!(
            slugify("Тайны Садового Товарищества"),
            "tajni-sadovogo-tovarishchestva"
        );
        assert_eq!(slugify("Медальон"), "medalon");
        assert_eq!(
            slugify("Не ходите, дети, по лесу гулять"),
            "ne-khodite-deti-po-lesu-gulyat"
        );
        assert_eq!(slugify("Я хочу, чтоб ты жила"), "ya-khochu-chtob-ti-zhila");
        assert_eq!(slugify("Pokémon: Café ~Mix~"), "pokemon-cafe-mix");
    }

    #[test]
    fn sitemap_locs() {
        let x = "<urlset><url><loc>https://a/novels/detail/x/</loc></url><url><loc> https://a/b?c=1&amp;d=2 </loc></url></urlset>";
        assert_eq!(
            locs(x),
            vec!["https://a/novels/detail/x/", "https://a/b?c=1&d=2"]
        );
    }

    #[test]
    fn defaults_parse() {
        let s: Sources = serde_json::from_str(DEFAULT).unwrap();
        assert!(s
            .sources
            .iter()
            .any(|x| x.id == "vngames" && x.index.is_some()));
    }
}
