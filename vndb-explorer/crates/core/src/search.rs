//! Offline search over the whole catalog.
//!
//! Tags: `must` (strict), `want` (preferred), `exclude`. Sub-tags count for their parents.
//! Relevance order: VNs having all wanted tags first, then by number of matched wanted tags
//! (descending), then alphabetically by shown title.

use crate::catalog::{format_date, image_url, Catalog, Vn};
use crate::lang::{is_cjk, rank};
use crate::tagtree::TagSet;
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LangMode {
    /// Available in any non-CJK language (default).
    #[default]
    Readable,
    /// Available in one of `langs`.
    List,
    /// No language filter.
    Any,
}

/// How a language must be available.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LangSource {
    /// Any way (ready release or patch) — the default for shelves.
    #[default]
    Any,
    /// A ready (non-patch) release: playable without patching.
    Ready,
    /// Through a patch over another release.
    Patch,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EroMode {
    /// Show everything, 18+ included (default).
    #[default]
    Show,
    /// Hide VNs with sexual content.
    Hide,
    /// Only VNs with sexual content.
    Only,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Sort {
    #[default]
    Relevance,
    Rating,
    Votes,
    Newest,
    Oldest,
    Title,
    Shortest,
    Longest,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default)]
pub struct SearchRequest {
    pub text: Option<String>,
    /// Tags: "g123", "123" or a name/alias.
    pub must: Vec<String>,
    pub want: Vec<String>,
    pub exclude: Vec<String>,
    /// Minimum wanted tags a VN needs (default 1 when `want` is given).
    pub min_match: Option<usize>,
    /// 0 = no spoiler tags, 1 = minor, 2 = all.
    pub spoiler: u8,
    /// Minimum tag strength (0-3).
    pub min_level: f32,
    pub lang_mode: LangMode,
    /// Ready release, patch or any way (for `Readable` and `List`).
    pub lang_source: LangSource,
    /// Preferred languages in order; also the filter in `List` mode.
    pub langs: Vec<String>,
    /// Count machine-translation-only languages as available.
    pub allow_mtl: bool,
    pub ero: EroMode,
    pub platforms: Vec<String>,
    pub length_min: Option<u32>,
    pub length_max: Option<u32>,
    pub year_from: Option<u32>,
    pub year_to: Option<u32>,
    pub min_votes: Option<u32>,
    pub min_rating: Option<f32>,
    pub freeware: Option<bool>,
    /// Only VNs with a known age rating up to this (e.g. 12 = "12+ and younger").
    pub max_age: Option<u8>,
    /// Hide cancelled projects.
    pub hide_cancelled: bool,
    /// Only VNs that are already released (no TBA / future dates).
    pub released_only: bool,
    /// Also return per-tag counts over all found VNs (for "how many are left" in tag pickers).
    pub tag_counts: bool,
    /// Restrict to these VN ids (personal lists).
    pub ids: Option<Vec<u32>>,
    pub sort: Sort,
    pub offset: usize,
    pub limit: usize,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            text: None,
            must: vec![],
            want: vec![],
            exclude: vec![],
            min_match: None,
            spoiler: 0,
            min_level: 0.0,
            lang_mode: LangMode::Readable,
            lang_source: LangSource::Any,
            langs: vec!["en".into()],
            allow_mtl: true,
            ero: EroMode::Show,
            platforms: vec![],
            length_min: None,
            length_max: None,
            year_from: None,
            year_to: None,
            min_votes: None,
            min_rating: None,
            freeware: None,
            max_age: None,
            hide_cancelled: false,
            released_only: false,
            tag_counts: false,
            ids: None,
            sort: Sort::Relevance,
            offset: 0,
            limit: 48,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LangBadge {
    pub lang: String,
    /// 0 official/original, 1 fan translation, 2 machine translation
    pub kind: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct Card {
    pub id: String,
    pub title: String,
    pub original: Option<String>,
    pub image: Option<String>,
    pub image_full: Option<String>,
    pub img_sexual: u16,
    pub img_violence: u16,
    pub rating: Option<f32>,
    pub votes: u32,
    pub released: Option<String>,
    pub length_minutes: Option<u32>,
    pub langs: Vec<LangBadge>,
    pub platforms: Vec<String>,
    pub ero: bool,
    pub freeware: bool,
    /// Age rating (0, 12, 16, 18 …) when known.
    pub minage: Option<u8>,
    /// Languages with a ready release / available through a patch.
    pub ready_langs: Vec<String>,
    pub patch_langs: Vec<String>,
    pub devstatus: u8,
    pub matched: Vec<u32>,
    pub missing: Vec<u32>,
    pub all_wanted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTag {
    pub id: u32,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub total: usize,
    pub offset: usize,
    pub items: Vec<Card>,
    pub must: Vec<ResolvedTag>,
    pub want: Vec<ResolvedTag>,
    pub exclude: Vec<ResolvedTag>,
    pub warnings: Vec<String>,
    /// With `tag_counts: true`: how many of the found VNs have each tag (sub-tags included).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag_counts: Option<std::collections::HashMap<u32, u32>>,
}

struct Compiled {
    must: Vec<TagSet>,
    want: Vec<(u32, TagSet)>,
    exclude: Vec<TagSet>,
    ero: Option<TagSet>,
    words: Vec<String>,
}

fn has(v: &Vn, set: &TagSet, req: &SearchRequest) -> bool {
    v.tags.iter().any(|t| {
        !t.lie && t.spoiler <= req.spoiler && t.rating >= req.min_level && set.contains(t.tag)
    })
}

pub fn card(v: &Vn, order: &[String], matched: Vec<u32>, missing: Vec<u32>) -> Card {
    let mut langs: Vec<LangBadge> = v
        .langs
        .iter()
        .map(|(l, k)| LangBadge {
            lang: l.clone(),
            kind: *k,
        })
        .collect();
    langs.sort_by_key(|b| (rank(order, &b.lang), b.kind, b.lang.clone()));
    Card {
        id: v.vid(),
        title: v.display_title(order),
        original: v.original_title(),
        image: v.image.as_deref().and_then(|i| image_url(i, true)),
        image_full: v.image.as_deref().and_then(|i| image_url(i, false)),
        img_sexual: v.img_sexual,
        img_violence: v.img_violence,
        rating: (v.votecount > 0).then_some(v.rating),
        votes: v.votecount,
        released: format_date(v.released),
        length_minutes: v.length_minutes,
        langs,
        platforms: v.platforms.clone(),
        ero: v.has_ero,
        freeware: v.freeware,
        minage: (v.minage >= 0).then_some(v.minage as u8),
        ready_langs: v.ready_langs.clone(),
        patch_langs: v.patch_langs.clone(),
        devstatus: v.devstatus,
        all_wanted: missing.is_empty(),
        matched,
        missing,
    }
}

fn lang_ok(v: &Vn, req: &SearchRequest, split: bool) -> bool {
    let usable = |k: u8| req.allow_mtl || k < 2;
    // without the split (an old database that could not be migrated) every language counts
    let source = if split {
        req.lang_source
    } else {
        LangSource::Any
    };
    let via = |l: &String| match source {
        LangSource::Any => true,
        LangSource::Ready => v.ready_langs.contains(l),
        LangSource::Patch => v.patch_langs.contains(l),
    };
    match req.lang_mode {
        LangMode::Any => true,
        LangMode::Readable => v
            .langs
            .iter()
            .any(|(l, k)| !is_cjk(l) && usable(*k) && via(l)),
        LangMode::List => v
            .langs
            .iter()
            .any(|(l, k)| req.langs.contains(l) && usable(*k) && via(l)),
    }
}

pub fn search(cat: &Catalog, req: &SearchRequest) -> Result<SearchResponse> {
    let mut warnings = Vec::new();
    let mut resolve = |list: &[String]| -> Vec<ResolvedTag> {
        let mut out = Vec::new();
        for s in list {
            match cat.tree.resolve(s) {
                Ok(id) => {
                    if !out.iter().any(|r: &ResolvedTag| r.id == id) {
                        out.push(ResolvedTag {
                            id,
                            name: cat.tree.get(id).map(|t| t.name.clone()).unwrap_or_default(),
                        })
                    }
                }
                Err(e) => warnings.push(e.to_string()),
            }
        }
        out
    };
    let must = resolve(&req.must);
    let want = resolve(&req.want);
    let exclude = resolve(&req.exclude);
    let comp = Compiled {
        must: must.iter().map(|t| cat.tree.subtree(t.id)).collect(),
        want: want
            .iter()
            .map(|t| (t.id, cat.tree.subtree(t.id)))
            .collect(),
        exclude: exclude.iter().map(|t| cat.tree.subtree(t.id)).collect(),
        ero: cat.ero_tag_root.map(|r| cat.tree.subtree(r)),
        words: req
            .text
            .as_deref()
            .unwrap_or("")
            .to_lowercase()
            .split_whitespace()
            .map(String::from)
            .collect(),
    };
    // Wanted tags only rank results unless a minimum is asked for explicitly.
    let min_match = req.min_match.unwrap_or(0);
    let ids: Option<std::collections::HashSet<u32>> =
        req.ids.as_ref().map(|v| v.iter().copied().collect());

    struct Hit<'a> {
        v: &'a Vn,
        matched: Vec<u32>,
        missing: Vec<u32>,
        title: String,
        text_score: u8,
    }
    let order = &req.langs;
    let today = today_yyyymmdd();
    let mut hits: Vec<Hit> = Vec::new();
    for v in &cat.vns {
        if let Some(ids) = &ids {
            if !ids.contains(&v.id) {
                continue;
            }
        }
        if !lang_ok(v, req, cat.lang_split) {
            continue;
        }
        let is_ero = v.has_ero
            || comp
                .ero
                .as_ref()
                .map(|e| {
                    v.tags
                        .iter()
                        .any(|t| !t.lie && t.rating >= 1.0 && e.contains(t.tag))
                })
                .unwrap_or(false);
        match req.ero {
            EroMode::Hide if is_ero => continue,
            EroMode::Only if !is_ero => continue,
            _ => {}
        }
        if req.hide_cancelled && v.devstatus == 2 {
            continue;
        }
        if req.released_only && (v.released == 0 || v.released > today) {
            continue;
        }
        if !req.platforms.is_empty() && !v.platforms.iter().any(|p| req.platforms.contains(p)) {
            continue;
        }
        if let Some(m) = req.length_min {
            if v.length_minutes.map(|l| l < m).unwrap_or(true) {
                continue;
            }
        }
        if let Some(m) = req.length_max {
            if v.length_minutes.map(|l| l > m).unwrap_or(true) {
                continue;
            }
        }
        let year = v.released / 10000;
        if let Some(y) = req.year_from {
            if year == 0 || year < y || v.released == 99999999 {
                continue;
            }
        }
        if let Some(y) = req.year_to {
            if year == 0 || year > y {
                continue;
            }
        }
        if req.min_votes.map(|m| v.votecount < m).unwrap_or(false) {
            continue;
        }
        if req
            .min_rating
            .map(|m| v.rating < m || v.votecount == 0)
            .unwrap_or(false)
        {
            continue;
        }
        if req
            .max_age
            .is_some_and(|m| v.minage < 0 || v.minage as u8 > m)
        {
            continue;
        }
        if req.freeware.map(|f| v.freeware != f).unwrap_or(false) {
            continue;
        }
        if !comp.must.iter().all(|s| has(v, s, req)) {
            continue;
        }
        if comp.exclude.iter().any(|s| has(v, s, req)) {
            continue;
        }
        let mut text_score = 0;
        if !comp.words.is_empty() {
            if !comp
                .words
                .iter()
                .all(|w| v.search_text.contains(w.as_str()))
            {
                continue;
            }
            let full = comp.words.join(" ");
            text_score = if v.search_text.lines().any(|l| l == full) {
                2
            } else if v.search_text.lines().any(|l| l.starts_with(&full)) {
                1
            } else {
                0
            };
        }
        let mut matched = Vec::new();
        let mut missing = Vec::new();
        for (id, s) in &comp.want {
            if has(v, s, req) {
                matched.push(*id)
            } else {
                missing.push(*id)
            }
        }
        if matched.len() < min_match {
            continue;
        }
        hits.push(Hit {
            v,
            title: v.display_title(order),
            matched,
            missing,
            text_score,
        });
    }

    let by_title = |a: &Hit, b: &Hit| a.title.to_lowercase().cmp(&b.title.to_lowercase());
    let rating = |h: &Hit| if h.v.votecount == 0 { 0.0 } else { h.v.rating };
    hits.sort_by(|a, b| match req.sort {
        Sort::Relevance => {
            if !comp.want.is_empty() {
                b.missing
                    .is_empty()
                    .cmp(&a.missing.is_empty())
                    .then(b.matched.len().cmp(&a.matched.len()))
                    .then_with(|| by_title(a, b))
            } else {
                b.text_score
                    .cmp(&a.text_score)
                    .then(b.v.votecount.cmp(&a.v.votecount))
                    .then_with(|| by_title(a, b))
            }
        }
        Sort::Rating => rating(b)
            .partial_cmp(&rating(a))
            .unwrap()
            .then(b.v.votecount.cmp(&a.v.votecount)),
        Sort::Votes => {
            b.v.votecount
                .cmp(&a.v.votecount)
                .then_with(|| by_title(a, b))
        }
        Sort::Newest => {
            let k = |h: &Hit| {
                if h.v.released == 99999999 {
                    0
                } else {
                    h.v.released
                }
            };
            k(b).cmp(&k(a)).then_with(|| by_title(a, b))
        }
        Sort::Oldest => {
            let k = |h: &Hit| {
                if h.v.released == 0 {
                    u32::MAX
                } else {
                    h.v.released
                }
            };
            k(a).cmp(&k(b)).then_with(|| by_title(a, b))
        }
        Sort::Title => by_title(a, b),
        Sort::Shortest => {
            a.v.length_minutes
                .unwrap_or(u32::MAX)
                .cmp(&b.v.length_minutes.unwrap_or(u32::MAX))
                .then_with(|| by_title(a, b))
        }
        Sort::Longest => {
            b.v.length_minutes
                .unwrap_or(0)
                .cmp(&a.v.length_minutes.unwrap_or(0))
                .then_with(|| by_title(a, b))
        }
    });

    let total = hits.len();
    let tag_counts = req.tag_counts.then(|| {
        let mut counts: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
        let mut seen: Vec<u32> = Vec::new();
        for h in &hits {
            seen.clear();
            for t in
                h.v.tags
                    .iter()
                    .filter(|t| !t.lie && t.spoiler <= req.spoiler && t.rating >= req.min_level)
            {
                if let Some(anc) = cat.ancestors.get(&t.tag) {
                    for a in anc {
                        if !seen.contains(a) {
                            seen.push(*a);
                        }
                    }
                }
            }
            for a in &seen {
                *counts.entry(*a).or_default() += 1;
            }
        }
        counts
    });
    let limit = req.limit.clamp(1, 500);
    let items = hits
        .into_iter()
        .skip(req.offset)
        .take(limit)
        .map(|h| card(h.v, order, h.matched, h.missing))
        .collect();
    Ok(SearchResponse {
        tag_counts,
        total,
        offset: req.offset,
        items,
        must,
        want,
        exclude,
        warnings,
    })
}

/// Current date as YYYYMMDD (UTC), without a date library.
pub fn today_yyyymmdd() -> u32 {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86400) as i64;
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y * 10000 + m * 100 + d) as u32
}
