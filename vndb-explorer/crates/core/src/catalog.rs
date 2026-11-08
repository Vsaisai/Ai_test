//! In-memory catalog of all visual novels, loaded from the local SQLite database.

use crate::lang::{has_cjk_chars, is_cjk, rank, LangKind};
use crate::tagtree::TagTree;
use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct Title {
    pub lang: String,
    pub title: String,
    pub latin: Option<String>,
    pub official: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct VnTag {
    pub tag: u32,
    pub rating: f32,
    pub spoiler: u8,
    pub lie: bool,
}

#[derive(Debug, Clone)]
pub struct Vn {
    pub id: u32,
    pub olang: String,
    pub titles: Vec<Title>,
    /// Lower-cased titles + aliases for text search.
    pub search_text: String,
    pub image: Option<String>,
    pub img_sexual: u16,
    pub img_violence: u16,
    pub votecount: u32,
    /// Bayesian rating 1-10.
    pub rating: f32,
    pub length_minutes: Option<u32>,
    pub length_cat: u8,
    pub devstatus: u8,
    pub released: u32,
    pub has_ero: bool,
    pub minage: i8,
    pub freeware: bool,
    pub platforms: Vec<String>,
    pub developers: Vec<u32>,
    pub langs: Vec<(String, LangKind)>,
    /// Languages with a ready (non-patch) release: playable without patching.
    pub ready_langs: Vec<String>,
    /// Languages available through a patch over another release.
    pub patch_langs: Vec<String>,
    pub tags: Vec<VnTag>,
}

pub struct Catalog {
    pub path: PathBuf,
    /// Languages are split into ready releases and patches (false if an old database could not
    /// be migrated: then the "how" filter is ignored and every language counts).
    pub lang_split: bool,
    pub tree: TagTree,
    pub vns: Vec<Vn>,
    pub by_id: HashMap<u32, usize>,
    pub producers: HashMap<u32, String>,
    pub dump_timestamp: String,
    pub imported_at: String,
    /// Tag ids under "Sexual Content" or with cat = ero.
    pub ero_tag_root: Option<u32>,
    /// Tag -> itself and all ancestors (for counting tags together with their sub-tags).
    pub ancestors: HashMap<u32, Vec<u32>>,
}

impl Vn {
    pub fn vid(&self) -> String {
        format!("v{}", self.id)
    }

    fn main_title(&self) -> Option<&Title> {
        self.titles
            .iter()
            .find(|t| t.lang == self.olang)
            .or(self.titles.first())
    }

    /// Best readable title according to the preferred language order.
    pub fn display_title(&self, order: &[String]) -> String {
        let mut ts: Vec<&Title> = self.titles.iter().collect();
        ts.sort_by_key(|t| (rank(order, &t.lang), !t.official));
        for t in &ts {
            if is_cjk(&t.lang) {
                continue;
            }
            if !has_cjk_chars(&t.title) {
                return t.title.clone();
            }
            if let Some(l) = &t.latin {
                return l.clone();
            }
        }
        if let Some(m) = self.main_title() {
            if let Some(l) = &m.latin {
                return l.clone();
            }
            return m.title.clone();
        }
        self.vid()
    }

    pub fn original_title(&self) -> Option<String> {
        self.main_title().map(|t| t.title.clone())
    }

    pub fn lang_kind(&self, lang: &str) -> Option<LangKind> {
        self.langs.iter().find(|(l, _)| l == lang).map(|(_, k)| *k)
    }
}

pub fn image_url(id: &str, thumb: bool) -> Option<String> {
    let prefix: String = id.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let num: u64 = id[prefix.len()..].parse().ok()?;
    let dir = match (prefix.as_str(), thumb) {
        ("cv", true) => "cv.t",
        ("sf", true) => "st",
        (p, _) => p,
    };
    Some(format!("{dir}/{:02}/{num}.jpg", num % 100))
}

pub fn format_date(d: u32) -> Option<String> {
    if d == 0 {
        return None;
    }
    if d == 99999999 {
        return Some("дата неизвестна".into());
    }
    let (y, m, day) = (d / 10000, d / 100 % 100, d % 100);
    Some(if m == 99 || m == 0 {
        format!("{y}")
    } else if day == 99 || day == 0 {
        format!("{y}-{m:02}")
    } else {
        format!("{y}-{m:02}-{day:02}")
    })
}

impl Catalog {
    pub fn open(path: &Path) -> Result<Self> {
        // add the ready/patch language columns to older databases; if that is not possible the
        // catalog still opens (every language then counts as ready)
        let lang_split = match crate::db::migrate_catalog(path) {
            Ok(()) => true,
            Err(e) => {
                eprintln!("language columns not added ({e:#}); patches are not told apart");
                false
            }
        };
        let c = crate::db::open_catalog(path)
            .with_context(|| format!("cannot open catalog {}", path.display()))?;
        let mut tree = TagTree::load(&c)?;
        let meta = |k: &str| -> String {
            c.query_row("SELECT value FROM meta WHERE key = ?1", [k], |r| r.get(0))
                .unwrap_or_default()
        };
        let dump_timestamp = meta("dump_timestamp");
        let imported_at = meta("imported_at");

        let mut vns: Vec<Vn> = Vec::new();
        let mut by_id = HashMap::new();
        {
            let mut st = c.prepare(
                "SELECT id, olang, image, img_sexual, img_violence, votecount, rating, length_minutes, length_cat, devstatus,
                        released, has_ero, minage, freeware, platforms, developers, aliases FROM vn ORDER BY id",
            )?;
            let rows = st.query_map([], |r| {
                let plats: String = r.get(14)?;
                let devs: String = r.get(15)?;
                let aliases: String = r.get(16)?;
                Ok((
                    Vn {
                        id: r.get(0)?,
                        olang: r.get(1)?,
                        titles: vec![],
                        search_text: String::new(),
                        image: r.get(2)?,
                        img_sexual: r.get(3)?,
                        img_violence: r.get(4)?,
                        votecount: r.get(5)?,
                        rating: r.get::<_, Option<f64>>(6)?.unwrap_or(0.0) as f32,
                        length_minutes: r.get(7)?,
                        length_cat: r.get(8)?,
                        devstatus: r.get(9)?,
                        released: r.get::<_, i64>(10)?.max(0) as u32,
                        has_ero: r.get(11)?,
                        minage: r.get::<_, i64>(12)?.clamp(-1, 99) as i8,
                        freeware: r.get(13)?,
                        platforms: plats.split_whitespace().map(String::from).collect(),
                        developers: devs
                            .split_whitespace()
                            .filter_map(|x| x.parse().ok())
                            .collect(),
                        langs: vec![],
                        ready_langs: vec![],
                        patch_langs: vec![],
                        tags: vec![],
                    },
                    aliases,
                ))
            })?;
            for row in rows {
                let (vn, aliases) = row?;
                by_id.insert(vn.id, vns.len());
                vns.push(vn);
                vns.last_mut().unwrap().search_text = aliases.to_lowercase();
            }
        }
        let idx = |id: u32| by_id.get(&id).copied();
        {
            let mut st = c.prepare("SELECT vid, lang, official, title, latin FROM vn_titles")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(i) = idx(r.get(0)?) {
                    let t = Title {
                        lang: r.get(1)?,
                        official: r.get(2)?,
                        title: r.get(3)?,
                        latin: r.get(4)?,
                    };
                    let v = &mut vns[i];
                    v.search_text.push('\n');
                    v.search_text.push_str(&t.title.to_lowercase());
                    if let Some(l) = &t.latin {
                        v.search_text.push('\n');
                        v.search_text.push_str(&l.to_lowercase());
                    }
                    v.titles.push(t);
                }
            }
        }
        {
            let mut st = c.prepare(if lang_split {
                "SELECT vid, lang, kind, ready, patch FROM vn_langs"
            } else {
                "SELECT vid, lang, kind, 0, 0 FROM vn_langs"
            })?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(i) = idx(r.get(0)?) {
                    let lang: String = r.get(1)?;
                    if r.get::<_, bool>(3)? {
                        vns[i].ready_langs.push(lang.clone());
                    }
                    if r.get::<_, bool>(4)? {
                        vns[i].patch_langs.push(lang.clone());
                    }
                    vns[i].langs.push((lang, r.get(2)?));
                }
            }
        }
        {
            let mut st = c.prepare("SELECT vid, tag, rating, spoiler, lie FROM vn_tags")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(i) = idx(r.get(0)?) {
                    vns[i].tags.push(VnTag {
                        tag: r.get(1)?,
                        rating: r.get::<_, f64>(2)? as f32,
                        spoiler: r.get(3)?,
                        lie: r.get(4)?,
                    });
                }
            }
        }
        for v in vns.iter_mut() {
            v.tags.sort_by(|a, b| {
                b.rating
                    .partial_cmp(&a.rating)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        let mut producers = HashMap::new();
        {
            let mut st = c.prepare("SELECT id, name, latin FROM producers")?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                let name: String = r.get(1)?;
                let latin: Option<String> = r.get(2)?;
                producers.insert(
                    r.get::<_, u32>(0)?,
                    latin.filter(|_| has_cjk_chars(&name)).unwrap_or(name),
                );
            }
        }
        // tag usage counts (tag itself or any descendant, non-lie, rating > 0)
        let mut counts: HashMap<u32, u32> = HashMap::new();
        let mut ancestors_cache: HashMap<u32, Vec<u32>> = HashMap::new();
        for v in &vns {
            let mut seen: Vec<u32> = Vec::new();
            for t in v.tags.iter().filter(|t| !t.lie) {
                let anc = ancestors_cache
                    .entry(t.tag)
                    .or_insert_with(|| ancestors(&tree, t.tag));
                for a in anc.iter() {
                    if !seen.contains(a) {
                        seen.push(*a);
                    }
                }
            }
            for a in seen {
                *counts.entry(a).or_default() += 1;
            }
        }
        for (id, n) in counts {
            if let Some(t) = tree.tags.get_mut(&id) {
                t.vns = n;
            }
        }
        let ero_tag_root = tree
            .tags
            .values()
            .find(|t| t.name == "Sexual Content")
            .map(|t| t.id);
        Ok(Self {
            path: path.to_path_buf(),
            lang_split,
            tree,
            vns,
            by_id,
            producers,
            dump_timestamp,
            imported_at,
            ero_tag_root,
            ancestors: ancestors_cache,
        })
    }

    pub fn get(&self, id: u32) -> Option<&Vn> {
        self.by_id.get(&id).map(|i| &self.vns[*i])
    }

    pub fn connection(&self) -> Result<Connection> {
        crate::db::open_catalog(&self.path)
    }
}

/// The tag itself plus every ancestor through all parent links.
fn ancestors(tree: &TagTree, id: u32) -> Vec<u32> {
    let mut out = Vec::new();
    let mut stack = vec![id];
    while let Some(c) = stack.pop() {
        if out.contains(&c) {
            continue;
        }
        out.push(c);
        if let Some(t) = tree.get(c) {
            stack.extend(t.parents.iter().copied());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates_and_images() {
        assert_eq!(format_date(20031017).unwrap(), "2003-10-17");
        assert_eq!(format_date(20039999).unwrap(), "2003");
        assert_eq!(format_date(20031099).unwrap(), "2003-10");
        assert_eq!(format_date(99999999).unwrap(), "дата неизвестна");
        assert_eq!(image_url("cv77859", false).unwrap(), "cv/59/77859.jpg");
        assert_eq!(image_url("cv77859", true).unwrap(), "cv.t/59/77859.jpg");
        assert_eq!(image_url("sf5707", true).unwrap(), "st/07/5707.jpg");
    }
}
