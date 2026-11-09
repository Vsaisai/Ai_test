//! Link external games to VNDB entries, so a game is shown once (merged duplicates).
//!
//! Signals, strongest first:
//! 1. **link** — a game-level official page (Steam app, itch.io game, DLsite product) is also a
//!    link of a VNDB release of exactly one VN;
//! 2. **title+link** — the title matches and a developer page (Patreon, SubscribeStar...) is
//!    linked by a release of that VN;
//! 3. **title+developer** — the title and the developer name match;
//! 4. **title** — the title matches exactly one adult VN.

use super::store::{self, Stored};
use super::{official_site, ExtGame};
use crate::catalog::Catalog;
use anyhow::Result;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

/// Lower-case words of a title or name, punctuation and accents removed.
pub fn norm(s: &str) -> String {
    crate::sources::slugify(s).replace('-', " ")
}

fn squash(s: &str) -> String {
    norm(s).replace(' ', "")
}

/// Comparable form of a URL: no scheme, `www.`, query, fragment or trailing slash.
pub fn norm_url(u: &str) -> String {
    let u = u.trim().to_lowercase();
    let u = u
        .strip_prefix("https://")
        .or_else(|| u.strip_prefix("http://"))
        .unwrap_or(&u)
        .to_string();
    let u = u.strip_prefix("www.").unwrap_or(&u);
    let u = u.split(['?', '#']).next().unwrap_or("");
    let u = u.trim_end_matches('/');
    // patreon.com/c/name and patreon.com/name are the same page
    u.replacen("patreon.com/c/", "patreon.com/", 1)
}

/// A page about one game (not a developer's page that covers all their games).
fn game_level(url: &str) -> bool {
    let n = norm_url(url);
    match official_site(url) {
        Some("steam") => n.contains("/app/"),
        Some("dlsite") => n.contains("/product_id/"),
        Some("gog") => n.contains("/game/"),
        // name.itch.io/game is a game, name.itch.io alone is the developer
        Some("itch") => n.split('/').filter(|p| !p.is_empty()).count() >= 2,
        _ => false,
    }
}

pub struct Index {
    by_title: HashMap<String, Vec<u32>>,
    by_url: HashMap<String, Vec<u32>>,
    devs: HashMap<u32, Vec<String>>,
    adult: HashSet<u32>,
}

impl Index {
    pub fn build(cat: &Catalog) -> Result<Self> {
        let mut by_title: HashMap<String, Vec<u32>> = HashMap::new();
        let mut devs = HashMap::new();
        let mut adult = HashSet::new();
        for vn in &cat.vns {
            let mut seen = HashSet::new();
            for t in &vn.titles {
                for s in std::iter::once(&t.title).chain(t.latin.iter()) {
                    let k = norm(s);
                    if !k.is_empty() && seen.insert(k.clone()) {
                        by_title.entry(k).or_default().push(vn.id);
                    }
                }
            }
            devs.insert(
                vn.id,
                vn.developers
                    .iter()
                    .filter_map(|p| cat.producers.get(p))
                    .map(|n| squash(n))
                    .collect(),
            );
            if vn.has_ero || vn.minage >= 18 {
                adult.insert(vn.id);
            }
        }
        let mut by_url: HashMap<String, Vec<u32>> = HashMap::new();
        let db =
            Connection::open_with_flags(&cat.path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let mut st = db.prepare(
            "SELECT DISTINCT rv.vid, rl.url FROM release_links rl JOIN release_vn rv ON rv.rid = rl.rid",
        )?;
        for r in st.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?)))? {
            let (vid, url) = r?;
            if official_site(&url).is_some() {
                let v = by_url.entry(norm_url(&url)).or_default();
                if !v.contains(&vid) {
                    v.push(vid);
                }
            }
        }
        Ok(Index {
            by_title,
            by_url,
            devs,
            adult,
        })
    }

    /// Build from plain data (tests).
    pub fn from_parts(
        titles: &[(&str, u32)],
        urls: &[(&str, u32)],
        devs: &[(u32, &str)],
        adult: &[u32],
    ) -> Self {
        let mut by_title: HashMap<String, Vec<u32>> = HashMap::new();
        for (t, v) in titles {
            by_title.entry(norm(t)).or_default().push(*v);
        }
        let mut by_url: HashMap<String, Vec<u32>> = HashMap::new();
        for (u, v) in urls {
            by_url.entry(norm_url(u)).or_default().push(*v);
        }
        let mut d: HashMap<u32, Vec<String>> = HashMap::new();
        for (v, n) in devs {
            d.entry(*v).or_default().push(squash(n));
        }
        Index {
            by_title,
            by_url,
            devs: d,
            adult: adult.iter().copied().collect(),
        }
    }

    pub fn find(&self, g: &ExtGame) -> Option<(u32, &'static str)> {
        // 1. the same game page
        for l in &g.links {
            if game_level(&l.url) {
                if let Some([vid]) = self.by_url.get(&norm_url(&l.url)).map(Vec::as_slice) {
                    return Some((*vid, "link"));
                }
            }
        }
        let candidates: Vec<u32> = self
            .by_title
            .get(&norm(&g.title))
            .cloned()
            .unwrap_or_default();
        if candidates.is_empty() {
            return None;
        }
        // 2. title + a developer page linked by that VN's releases
        let linked: HashSet<u32> = g
            .links
            .iter()
            .filter_map(|l| self.by_url.get(&norm_url(&l.url)))
            .flatten()
            .copied()
            .collect();
        let hits: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|v| linked.contains(v))
            .collect();
        if let [vid] = hits.as_slice() {
            return Some((*vid, "title+link"));
        }
        // 3. title + developer name
        let dev = squash(&g.developer);
        if !dev.is_empty() {
            let hits: Vec<u32> = candidates
                .iter()
                .copied()
                .filter(|v| self.devs.get(v).is_some_and(|d| d.contains(&dev)))
                .collect();
            if let [vid] = hits.as_slice() {
                return Some((*vid, "title+developer"));
            }
        }
        // 4. the only adult VN with this title
        let adult: Vec<u32> = candidates
            .iter()
            .copied()
            .filter(|v| self.adult.contains(v))
            .collect();
        if let [vid] = adult.as_slice() {
            return Some((*vid, "title"));
        }
        None
    }
}

/// Re-match every stored game; returns how many are linked to VNDB.
pub fn match_all(conn: &Connection, idx: &Index) -> Result<usize> {
    let games: Vec<Stored> = store::all(conn)?;
    let tx = conn.unchecked_transaction()?;
    let mut matched = 0;
    for s in &games {
        let m = idx.find(&s.game);
        if m.is_some() {
            matched += 1;
        }
        let (vid, kind) = (m.map(|x| x.0), m.map(|x| x.1));
        if vid != s.vndb_id || kind != s.match_kind.as_deref() {
            store::set_match(&tx, &s.game.source, &s.game.ext_id, vid, kind)?;
        }
    }
    tx.commit()?;
    Ok(matched)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::external::OfficialLink;

    fn g(title: &str, dev: &str, links: &[&str]) -> ExtGame {
        ExtGame {
            title: title.into(),
            developer: dev.into(),
            links: links
                .iter()
                .map(|u| OfficialLink {
                    site: official_site(u).unwrap_or("").into(),
                    label: String::new(),
                    url: u.to_string(),
                })
                .collect(),
            ..Default::default()
        }
    }

    #[test]
    fn urls_compare_loosely() {
        assert_eq!(
            norm_url("https://www.Patreon.com/c/DevName/?utm=1"),
            "patreon.com/devname"
        );
        assert!(game_level("https://dev.itch.io/my-game"));
        assert!(!game_level("https://dev.itch.io/"));
        assert!(game_level("https://store.steampowered.com/app/123/X/"));
        assert!(!game_level("https://www.patreon.com/dev"));
    }

    #[test]
    fn strongest_signal_wins_and_ambiguity_is_left_alone() {
        let idx = Index::from_parts(
            &[
                ("Summer Days", 1),
                ("Summer Days", 2),
                ("Lonely Title", 3),
                ("Lonely Title", 4),
            ],
            &[
                ("https://store.steampowered.com/app/555/", 2),
                ("https://www.patreon.com/acme", 1),
                ("https://www.patreon.com/acme", 9),
            ],
            &[(1, "Acme Games"), (2, "Other Studio")],
            &[1, 2, 3],
        );
        // same Steam app page
        assert_eq!(
            idx.find(&g(
                "Totally different name",
                "",
                &["https://store.steampowered.com/app/555"]
            )),
            Some((2, "link"))
        );
        // two VNs share the title: the developer page decides
        assert_eq!(
            idx.find(&g("Summer Days!", "", &["https://patreon.com/c/acme"])),
            Some((1, "title+link"))
        );
        // ...or the developer name
        assert_eq!(
            idx.find(&g("summer days", "OtherStudio", &[])),
            Some((2, "title+developer"))
        );
        // ambiguous title, nothing else: no match
        assert_eq!(idx.find(&g("Summer Days", "Nobody", &[])), None);
        // the only adult VN with this title
        assert_eq!(idx.find(&g("Lonely Title", "", &[])), Some((3, "title")));
        assert_eq!(idx.find(&g("Unknown", "", &[])), None);
    }
}
