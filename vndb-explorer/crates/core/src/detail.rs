//! Full information for one VN, including categorized tags and where to get it.

use crate::catalog::{format_date, image_url, Catalog, Title};
use crate::lang::{is_cjk, rank};
use crate::search::{card, Card};
use anyhow::{Context, Result};
use rusqlite::params;
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, Serialize)]
pub struct TagItem {
    pub id: u32,
    pub name: String,
    pub rating: f32,
    pub spoiler: u8,
    pub ero: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagGroup {
    pub name: String,
    pub tags: Vec<TagItem>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TagCategory {
    pub name: String,
    pub groups: Vec<TagGroup>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ReleaseLang {
    pub lang: String,
    pub mtl: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Link {
    pub site: String,
    pub label: String,
    pub url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Release {
    pub id: String,
    pub title: String,
    pub released: Option<String>,
    pub rtype: String,
    pub langs: Vec<ReleaseLang>,
    pub platforms: Vec<String>,
    pub official: bool,
    pub patch: bool,
    pub freeware: bool,
    pub minage: i64,
    pub has_ero: bool,
    pub uncensored: bool,
    pub producers: Vec<String>,
    pub links: Vec<Link>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Official free download (itch.io, Game Jolt, the author's site ...)
    Free,
    /// Store page
    Store,
    /// Buy directly from developer / publisher
    Direct,
    /// Fan translation / patch
    FanPatch,
    /// Nothing known: search links
    Search,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub kind: SourceKind,
    pub site: String,
    pub url: String,
    pub release: Option<String>,
    pub langs: Vec<String>,
    pub mtl: bool,
    pub platforms: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Screenshot {
    pub thumb: String,
    pub full: String,
    pub sexual: u16,
    pub violence: u16,
}

#[derive(Debug, Clone, Serialize)]
pub struct Relation {
    pub id: String,
    pub title: String,
    pub relation: String,
    pub official: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct Detail {
    #[serde(flatten)]
    pub card: Card,
    pub titles: Vec<Title>,
    pub aliases: Vec<String>,
    pub description: String,
    /// The description split into text and links (spoiler markers stay inside the text).
    pub description_parts: Vec<DescPart>,
    pub developers: Vec<String>,
    pub length_votes: u32,
    pub tags: Vec<TagCategory>,
    pub screenshots: Vec<Screenshot>,
    pub relations: Vec<Relation>,
    pub releases: Vec<Release>,
    pub sources: Vec<Source>,
    pub links: Vec<Link>,
    pub vndb_url: String,
}

/// (lang, title, latin)
type RelTitle = (String, Option<String>, Option<String>);

const DOWNLOAD_SITES: &[&str] = &[
    "itch", "gamejolt", "freem", "freegame", "novelgam", "booth", "patreonp", "renai",
];
const STORES: &[&str] = &[
    "steam",
    "gog",
    "itch",
    "jastusa",
    "mg",
    "denpa",
    "fakku",
    "dlsite",
    "dlsiteen",
    "nutaku",
    "johren",
    "kagura",
    "jlist",
    "playasia",
    "googplay",
    "appstore",
    "nintendo",
    "nintendo_jp",
    "nintendo_hk",
    "playstation_na",
    "playstation_eu",
    "playstation_jp",
    "playstation_hk",
    "dmm",
    "getchudl",
    "getchu",
    "gyutto",
    "digiket",
    "toranoana",
    "melon",
    "melonjp",
    "animateg",
    "patreon",
    "substar",
    "booth",
];

fn site_rank(url: &str) -> u8 {
    let u = url.to_lowercase();
    if u.contains("itch.io") {
        0
    } else if [
        "gamejolt.com",
        "freem.ne.jp",
        "booth.pm",
        "patreon.com",
        "renai.us",
    ]
    .iter()
    .any(|d| u.contains(d))
    {
        1
    } else if ["steampowered.com", "gog.com"]
        .iter()
        .any(|d| u.contains(d))
    {
        2
    } else {
        3
    }
}

/// Classify release links into download / store / direct / fan-patch groups.
pub fn classify(
    releases: &[Release],
    order: &[String],
    accept: &dyn Fn(&str) -> bool,
) -> Vec<Source> {
    let mut out: Vec<Source> = Vec::new();
    let mut seen = HashSet::new();
    for r in releases {
        let langs: Vec<&ReleaseLang> = r.langs.iter().filter(|l| accept(&l.lang)).collect();
        if langs.is_empty() {
            continue;
        }
        let mut lnames: Vec<String> = langs.iter().map(|l| l.lang.clone()).collect();
        lnames.sort_by_key(|l| (rank(order, l), l.clone()));
        let mtl = langs.iter().all(|l| l.mtl);
        for l in &r.links {
            let s = l.site.as_str();
            let known = s == "website" || DOWNLOAD_SITES.contains(&s) || STORES.contains(&s);
            let kind = if !known {
                continue;
            } else if !r.official {
                SourceKind::FanPatch
            } else if r.freeware {
                SourceKind::Free
            } else if STORES.contains(&s) {
                SourceKind::Store
            } else if s == "website" {
                SourceKind::Direct
            } else {
                SourceKind::Store
            };
            if !seen.insert(l.url.clone()) {
                continue;
            }
            let site = if kind == SourceKind::FanPatch && s == "website" {
                "Сайт переводчиков".to_string()
            } else {
                l.label.clone()
            };
            let mut note = r.title.clone();
            if let Some(d) = &r.released {
                note.push_str(&format!(" ({d})"));
            }
            out.push(Source {
                kind,
                site,
                url: l.url.clone(),
                release: Some(note),
                langs: lnames.clone(),
                mtl,
                platforms: r.platforms.clone(),
            });
        }
    }
    out.sort_by_key(|s| {
        (
            s.kind,
            s.mtl,
            s.langs.first().map(|l| rank(order, l)).unwrap_or(500),
            site_rank(&s.url),
            s.site.to_lowercase(),
        )
    });
    out
}

pub fn fallback(title: &str) -> Vec<Source> {
    let q = urlencoding::encode(title);
    let mk = |site: &str, url: String| Source {
        kind: SourceKind::Search,
        site: site.into(),
        url,
        release: None,
        langs: vec![],
        mtl: false,
        platforms: vec![],
    };
    vec![
        mk("itch.io", format!("https://itch.io/search?q={q}")),
        mk(
            "Steam",
            format!("https://store.steampowered.com/search/?term={q}"),
        ),
        mk("GOG", format!("https://www.gog.com/en/games?query={q}")),
        mk(
            "DLsite",
            format!("https://www.dlsite.com/home/fsr/=/keyword/{q}"),
        ),
    ]
}

/// A screenshot linked from a tag description ("Example 1", "NSFW Example 2").
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TagExample {
    /// Image path for the `/img/` proxy, e.g. `sf/01/6101.jpg`.
    pub path: String,
    pub nsfw: bool,
}

/// A piece of a tag description: plain text, a link to another tag, or an external link.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct DescPart {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

impl DescPart {
    fn is_text(&self) -> bool {
        self.tag.is_none() && self.vn.is_none() && self.url.is_none()
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct TagDescription {
    /// Plain text without markup and without the example links (for translation).
    pub text: String,
    pub parts: Vec<DescPart>,
    pub examples: Vec<TagExample>,
}

fn screenshot_path(url: &str) -> Option<String> {
    let rest = url.split("vndb.org/").nth(1)?;
    let p = rest.trim_end_matches('/');
    p.starts_with("sf/").then(|| p.to_string())
}

/// `/g12`, `https://vndb.org/v5` -> ('g', 12)
fn vndb_link(url: &str) -> Option<(char, u32)> {
    let rest = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("www.")
        .trim_start_matches("vndb.org");
    let rest = rest.strip_prefix('/')?.trim_end_matches('/');
    let kind = rest.chars().next()?;
    let id = rest[1..].split(['#', '?', '/', '.']).next()?.parse().ok()?;
    Some((kind, id))
}

/// Split a VNDB tag description into readable parts: screenshot links ("Example 1") become
/// image examples instead of dangling words, `/gN` links become links to tags.
pub fn tag_description(raw: &str) -> TagDescription {
    let mut parts: Vec<DescPart> = Vec::new();
    let mut examples = Vec::new();
    let push =
        |parts: &mut Vec<DescPart>, text: &str, link: Option<(char, u32)>, url: Option<String>| {
            if text.is_empty() {
                return;
            }
            let (tag, vn) = match link {
                Some(('g', id)) => (Some(id), None),
                Some(('v', id)) => (None, Some(id)),
                _ => (None, None),
            };
            if tag.is_none() && vn.is_none() && url.is_none() {
                if let Some(last) = parts.last_mut().filter(|p| p.is_text()) {
                    last.text.push_str(text);
                    return;
                }
            }
            parts.push(DescPart {
                text: text.to_string(),
                tag,
                vn,
                url,
            });
        };
    let mut rest = raw;
    loop {
        let Some(start) = rest.find("[url=") else {
            push(&mut parts, &clean_bbcode_keep(rest), None, None);
            break;
        };
        push(&mut parts, &clean_bbcode_keep(&rest[..start]), None, None);
        let after = &rest[start + 5..];
        let (Some(close), Some(end)) = (after.find(']'), after.find("[/url]")) else {
            push(&mut parts, &clean_bbcode_keep(after), None, None);
            break;
        };
        if close > end {
            rest = &after[end + 6..];
            continue;
        }
        let url = after[..close].trim();
        let text = clean_bbcode_keep(&after[close + 1..end]);
        rest = &after[end + 6..];
        if let Some(path) = screenshot_path(url) {
            examples.push(TagExample {
                path,
                nsfw: text.to_lowercase().contains("nsfw"),
            });
        } else if let Some(l @ ('g' | 'v', _)) = vndb_link(url) {
            push(&mut parts, &text, Some(l), None);
        } else if url.starts_with("http") {
            push(&mut parts, &text, None, Some(url.to_string()));
        } else if url.starts_with('/') && vndb_link(url).is_some() {
            push(
                &mut parts,
                &text,
                None,
                Some(format!("https://vndb.org{url}")),
            );
        } else {
            push(&mut parts, &text, None, None);
        }
    }
    // tidy whitespace left by removed example links: no blank-ish lines, at most one empty line
    for p in parts.iter_mut().filter(|p| p.is_text()) {
        let mut t: String = p
            .text
            .split('\n')
            .map(|l| if l.trim().is_empty() { "" } else { l })
            .collect::<Vec<_>>()
            .join("\n");
        while t.contains("\n\n\n") {
            t = t.replace("\n\n\n", "\n\n");
        }
        p.text = t;
    }
    if let Some(p) = parts.first_mut() {
        p.text = p.text.trim_start().to_string();
    }
    if let Some(p) = parts.last_mut() {
        p.text = p.text.trim_end().to_string();
    }
    parts.retain(|p| !p.text.is_empty());
    let text: String = parts.iter().map(|p| p.text.as_str()).collect();
    TagDescription {
        text: text.trim().to_string(),
        parts,
        examples,
    }
}

/// Like [`clean_bbcode`] but keeps the surrounding whitespace (used for fragments).
fn clean_bbcode_keep(s: &str) -> String {
    let lead = s.len() - s.trim_start().len();
    let trail = s.len() - s.trim_end().len();
    let core = clean_bbcode(s);
    if core.is_empty() {
        return s.to_string();
    }
    format!("{}{}{}", &s[..lead], core, &s[s.len() - trail..])
}

/// Cleaned description (VNDB BBCode stripped, spoilers kept in `[spoiler]` markers for the UI).
pub fn clean_bbcode(s: &str) -> String {
    const KNOWN: &[&str] = &["b", "i", "u", "s", "url", "quote", "raw", "code"];
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find(']').map(|e| (e, &after[..e])) {
            Some((e, tag))
                if KNOWN.contains(
                    &tag.trim_start_matches('/')
                        .split('=')
                        .next()
                        .unwrap_or("")
                        .to_lowercase()
                        .as_str(),
                ) =>
            {
                rest = &after[e + 1..];
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

pub fn detail(
    cat: &Catalog,
    id: u32,
    order: &[String],
    accept_any: bool,
) -> Result<Option<Detail>> {
    let Some(v) = cat.get(id) else {
        return Ok(None);
    };
    let c = cat.connection()?;
    let (aliases, description, length_votes): (String, String, u32) = c
        .query_row(
            "SELECT aliases, description, length_votes FROM vn WHERE id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .context("vn row")?;

    // tags grouped Root > Sub-category
    let mut groups: BTreeMap<(usize, String), BTreeMap<String, Vec<TagItem>>> = BTreeMap::new();
    for t in v.tags.iter().filter(|t| !t.lie) {
        let Some(tag) = cat.tree.get(t.tag) else {
            continue;
        };
        let (root, sub) = cat.tree.category(t.tag);
        let ro = crate::tagtree::ROOT_ORDER
            .iter()
            .position(|r| *r == root)
            .unwrap_or(99);
        let ero = tag.cat == "ero";
        groups
            .entry((ro, root))
            .or_default()
            .entry(sub)
            .or_default()
            .push(TagItem {
                id: t.tag,
                name: tag.name.clone(),
                rating: t.rating,
                spoiler: t.spoiler,
                ero,
            });
    }
    let tags = groups
        .into_iter()
        .map(|((_, name), subs)| {
            let mut gs: Vec<TagGroup> = subs
                .into_iter()
                .map(|(name, mut tags)| {
                    tags.sort_by(|a, b| {
                        b.rating
                            .partial_cmp(&a.rating)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                    TagGroup { name, tags }
                })
                .collect();
            gs.sort_by(|a, b| {
                (a.name == "General")
                    .cmp(&(b.name == "General"))
                    .reverse()
                    .then(a.name.cmp(&b.name))
            });
            TagCategory { name, groups: gs }
        })
        .collect();

    // releases
    let mut rel_map: HashMap<u32, Release> = HashMap::new();
    let mut rel_order: Vec<(u32, i64)> = Vec::new();
    {
        let mut st = c.prepare(
            "SELECT r.id, rv.rtype, r.released, r.minage, r.has_ero, r.patch, r.freeware, r.uncensored, r.official
             FROM release_vn rv JOIN releases r ON r.id = rv.rid WHERE rv.vid = ?1",
        )?;
        let mut rows = st.query([id])?;
        while let Some(r) = rows.next()? {
            let rid: u32 = r.get(0)?;
            let released: i64 = r.get(2)?;
            rel_order.push((rid, released));
            rel_map.insert(
                rid,
                Release {
                    id: format!("r{rid}"),
                    title: String::new(),
                    released: format_date(released.max(0) as u32),
                    rtype: r.get(1)?,
                    langs: vec![],
                    platforms: vec![],
                    official: r.get(8)?,
                    patch: r.get(5)?,
                    freeware: r.get(6)?,
                    minage: r.get(3)?,
                    has_ero: r.get(4)?,
                    uncensored: r.get(7)?,
                    producers: vec![],
                    links: vec![],
                },
            );
        }
        let ids: Vec<String> = rel_map.keys().map(|k| k.to_string()).collect();
        if !ids.is_empty() {
            let list = ids.join(",");
            let mut st = c.prepare(&format!(
                "SELECT rid, lang, mtl, title, latin FROM release_titles WHERE rid IN ({list})"
            ))?;
            let mut rows = st.query([])?;
            let mut titles: HashMap<u32, Vec<RelTitle>> = HashMap::new();
            while let Some(r) = rows.next()? {
                let rid: u32 = r.get(0)?;
                let lang: String = r.get(1)?;
                if let Some(rel) = rel_map.get_mut(&rid) {
                    rel.langs.push(ReleaseLang {
                        lang: lang.clone(),
                        mtl: r.get(2)?,
                    });
                }
                titles
                    .entry(rid)
                    .or_default()
                    .push((lang, r.get(3)?, r.get(4)?));
            }
            for (rid, ts) in titles {
                let mut ts = ts;
                ts.sort_by_key(|(l, _, _)| rank(order, l));
                let pick = ts
                    .iter()
                    .find_map(|(l, t, lat)| {
                        if is_cjk(l) {
                            lat.clone()
                        } else {
                            t.clone()
                                .filter(|t| !crate::lang::has_cjk_chars(t))
                                .or(lat.clone())
                        }
                    })
                    .or_else(|| ts.iter().find_map(|(_, t, _)| t.clone()));
                if let Some(rel) = rel_map.get_mut(&rid) {
                    rel.title = pick.unwrap_or_default();
                }
            }
            let mut st = c.prepare(&format!(
                "SELECT rid, platform FROM release_platforms WHERE rid IN ({list})"
            ))?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(rel) = rel_map.get_mut(&r.get(0)?) {
                    rel.platforms.push(r.get(1)?);
                }
            }
            let mut st = c.prepare(&format!(
                "SELECT rid, site, url FROM release_links WHERE rid IN ({list})"
            ))?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(rel) = rel_map.get_mut(&r.get(0)?) {
                    let site: String = r.get(1)?;
                    rel.links.push(Link {
                        label: crate::extlinks::label(&site).to_string(),
                        site,
                        url: r.get(2)?,
                    });
                }
            }
            let mut st = c.prepare(&format!(
                "SELECT rp.rid, p.name, p.latin FROM release_producers rp JOIN producers p ON p.id = rp.pid WHERE rp.rid IN ({list})"
            ))?;
            let mut rows = st.query([])?;
            while let Some(r) = rows.next()? {
                if let Some(rel) = rel_map.get_mut(&r.get(0)?) {
                    let name: String = r.get(1)?;
                    let latin: Option<String> = r.get(2)?;
                    rel.producers.push(
                        latin
                            .filter(|_| crate::lang::has_cjk_chars(&name))
                            .unwrap_or(name),
                    );
                }
            }
        }
    }
    rel_order.sort_by_key(|(id, d)| (if *d == 0 { i64::MAX } else { *d }, *id));
    let releases: Vec<Release> = rel_order
        .iter()
        .filter_map(|(id, _)| rel_map.remove(id))
        .collect();

    let title = v.display_title(order);
    let accept = |l: &str| accept_any || !is_cjk(l);
    let mut sources = classify(&releases, order, &accept);
    // developer / publisher homepages for commercial releases -> "buy direct"
    if sources
        .iter()
        .any(|s| matches!(s.kind, SourceKind::Store | SourceKind::Direct))
    {
        let mut st = c.prepare(
            "SELECT DISTINCT p.name, p.latin, p.website FROM release_vn rv
             JOIN releases r ON r.id = rv.rid JOIN release_producers rp ON rp.rid = r.id JOIN producers p ON p.id = rp.pid
             WHERE rv.vid = ?1 AND r.official AND NOT r.freeware AND p.website IS NOT NULL
               AND (?2 OR EXISTS (SELECT 1 FROM release_titles rt WHERE rt.rid = r.id AND rt.lang NOT IN ('ja','zh','zh-Hans','zh-Hant','ko')))",
        )?;
        let rows = st.query_map(params![id, accept_any], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        let seen: HashSet<String> = sources.iter().map(|s| s.url.clone()).collect();
        for row in rows {
            let (name, latin, url) = row?;
            if seen.contains(&url) {
                continue;
            }
            let name = latin
                .filter(|_| crate::lang::has_cjk_chars(&name))
                .unwrap_or(name);
            sources.push(Source {
                kind: SourceKind::Direct,
                site: format!("{name} — сайт"),
                url,
                release: None,
                langs: vec![],
                mtl: false,
                platforms: vec![],
            });
        }
    }
    if !sources
        .iter()
        .any(|s| matches!(s.kind, SourceKind::Free | SourceKind::Store))
    {
        sources.extend(fallback(&title));
    }

    let mut screenshots = Vec::new();
    {
        let mut st =
            c.prepare("SELECT scr, sexual, violence FROM vn_screenshots WHERE vid = ?1")?;
        let rows = st.query_map([id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, u16>(1)?,
                r.get::<_, u16>(2)?,
            ))
        })?;
        for row in rows {
            let (scr, s, vi) = row?;
            if let (Some(t), Some(f)) = (image_url(&scr, true), image_url(&scr, false)) {
                screenshots.push(Screenshot {
                    thumb: t,
                    full: f,
                    sexual: s,
                    violence: vi,
                });
            }
        }
    }
    let mut relations = Vec::new();
    {
        let mut st =
            c.prepare("SELECT other, relation, official FROM vn_relations WHERE vid = ?1")?;
        let rows = st.query_map([id], |r| {
            Ok((
                r.get::<_, u32>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, bool>(2)?,
            ))
        })?;
        for row in rows {
            let (o, rel, off) = row?;
            if let Some(ov) = cat.get(o) {
                relations.push(Relation {
                    id: ov.vid(),
                    title: ov.display_title(order),
                    relation: rel,
                    official: off,
                });
            }
        }
    }
    let mut links = Vec::new();
    {
        let mut st = c.prepare("SELECT site, url FROM vn_links WHERE vid = ?1")?;
        let rows = st.query_map([id], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (site, url) = row?;
            links.push(Link {
                label: crate::extlinks::label(&site).to_string(),
                site,
                url,
            });
        }
    }

    Ok(Some(Detail {
        card: card(v, order, vec![], vec![]),
        titles: v.titles.clone(),
        aliases: aliases
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect(),
        description: clean_bbcode(&description),
        description_parts: tag_description(&description).parts,
        developers: v
            .developers
            .iter()
            .filter_map(|p| cat.producers.get(p).cloned())
            .collect(),
        length_votes,
        tags,
        screenshots,
        relations,
        releases,
        sources,
        links,
        vndb_url: format!("https://vndb.org/{}", v.vid()),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(official: bool, freeware: bool, lang: &str, links: &[(&str, &str)]) -> Release {
        Release {
            id: "r1".into(),
            title: "T".into(),
            released: None,
            rtype: "complete".into(),
            langs: vec![ReleaseLang {
                lang: lang.into(),
                mtl: false,
            }],
            platforms: vec![],
            official,
            patch: !official,
            freeware,
            minage: 0,
            has_ero: false,
            uncensored: false,
            producers: vec![],
            links: links
                .iter()
                .map(|(s, u)| Link {
                    site: s.to_string(),
                    label: s.to_string(),
                    url: u.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn classification_order() {
        let rels = vec![
            rel(
                true,
                false,
                "en",
                &[
                    ("steam", "https://store.steampowered.com/app/1/"),
                    ("website", "https://dev/buy"),
                ],
            ),
            rel(false, true, "ru", &[("website", "https://fans/")]),
            rel(true, true, "en", &[("itch", "https://x.itch.io/demo")]),
            rel(true, false, "ja", &[("dlsite", "https://dlsite/")]),
        ];
        let order = vec!["en".to_string()];
        let s = classify(&rels, &order, &|l: &str| !is_cjk(l));
        let got: Vec<(SourceKind, &str)> = s.iter().map(|s| (s.kind, s.url.as_str())).collect();
        assert_eq!(
            got,
            vec![
                (SourceKind::Free, "https://x.itch.io/demo"),
                (SourceKind::Store, "https://store.steampowered.com/app/1/"),
                (SourceKind::Direct, "https://dev/buy"),
                (SourceKind::FanPatch, "https://fans/"),
            ]
        );
    }

    #[test]
    fn tag_description_examples_and_links() {
        let d = tag_description("Box at the bottom.\n\nIf not, use the [url=http://vndb.org/g398]Floating Textbox[/url] tag.\n\n[url=http://s.vndb.org/sf/01/6101.jpg]Example 1[/url]\n[url=https://s2.vndb.org/sf/04/1304.jpg]NSFW Example 2[/url]\nSee [url=https://en.wikipedia.org/wiki/X]Wikipedia[/url].");
        assert_eq!(
            d.text,
            "Box at the bottom.\n\nIf not, use the Floating Textbox tag.\n\nSee Wikipedia."
        );
        assert_eq!(
            d.examples,
            vec![
                TagExample {
                    path: "sf/01/6101.jpg".into(),
                    nsfw: false
                },
                TagExample {
                    path: "sf/04/1304.jpg".into(),
                    nsfw: true
                },
            ]
        );
        assert_eq!(d.parts[1].tag, Some(398));
        assert_eq!(
            d.parts[3].url.as_deref(),
            Some("https://en.wikipedia.org/wiki/X")
        );
        assert!(!d.text.contains("Example"));
        let v = tag_description("Sequel to [url=/v17]Ever17[/url], see [url=/p12]the studio[/url]. [spoiler]x[/spoiler]");
        assert_eq!(v.parts[1].vn, Some(17));
        assert_eq!(v.parts[3].url.as_deref(), Some("https://vndb.org/p12"));
        assert!(v.text.ends_with("[spoiler]x[/spoiler]"));
    }

    #[test]
    fn bbcode() {
        assert_eq!(
            clean_bbcode("[From [url=https://w]Wikipedia[/url]]"),
            "[From Wikipedia]"
        );
        assert_eq!(
            clean_bbcode("a [b]b[/b] [spoiler]x[/spoiler]"),
            "a b [spoiler]x[/spoiler]"
        );
    }
}
