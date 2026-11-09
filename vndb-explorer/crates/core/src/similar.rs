//! "Similar novels": cosine similarity of tag vectors, rarer tags weigh more.

use crate::catalog::{Catalog, Vn};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Count sexual-content tags (only when the user shows adult content)
    pub adult_tags: bool,
    /// Leave out VNs with sexual content
    pub hide_ero: bool,
    /// At least this many votes (so the list has known titles)
    pub min_votes: u32,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            adult_tags: false,
            hide_ero: false,
            min_votes: 10,
        }
    }
}

/// Tag weights of one VN: rating × rarity, spoilers, lies and technical tags left out.
fn vector(cat: &Catalog, v: &Vn, idf: &HashMap<u32, f32>, opts: &Options) -> HashMap<u32, f32> {
    v.tags
        .iter()
        .filter(|t| t.rating >= 1.0 && t.spoiler == 0 && !t.lie)
        .filter_map(|t| {
            let tag = cat.tree.tags.get(&t.tag)?;
            if tag.cat == "tech" || (tag.cat == "ero" && !opts.adult_tags) {
                return None;
            }
            Some((t.tag, t.rating * idf.get(&t.tag).copied().unwrap_or(0.0)))
        })
        .collect()
}

fn norm(v: &HashMap<u32, f32>) -> f32 {
    v.values().map(|x| x * x).sum::<f32>().sqrt()
}

/// Up to `limit` VNs most similar to `vid`, best first, with their score (0..1).
pub fn similar(
    cat: &Catalog,
    vid: u32,
    limit: usize,
    opts: &Options,
    exclude: &std::collections::HashSet<u32>,
) -> Vec<(u32, f32)> {
    let Some(target) = cat.get(vid) else {
        return Vec::new();
    };
    let n = cat.vns.len().max(1) as f32;
    let idf: HashMap<u32, f32> = cat
        .tree
        .tags
        .iter()
        .map(|(id, t)| (*id, (n / (t.vns.max(1) as f32)).ln().max(0.0)))
        .collect();
    let tv = vector(cat, target, &idf, opts);
    let tn = norm(&tv);
    if tn == 0.0 {
        return Vec::new();
    }
    let mut scored: Vec<(u32, f32)> = cat
        .vns
        .iter()
        .filter(|v| {
            v.id != vid
                && !exclude.contains(&v.id)
                && v.votecount >= opts.min_votes
                && !(opts.hide_ero && v.has_ero)
        })
        .filter(|v| v.tags.iter().any(|t| tv.contains_key(&t.tag)))
        .filter_map(|v| {
            let cv = vector(cat, v, &idf, opts);
            let dot: f32 = cv
                .iter()
                .filter_map(|(k, w)| tv.get(k).map(|x| x * w))
                .sum();
            let cn = norm(&cv);
            (dot > 0.0 && cn > 0.0).then(|| {
                // a small nudge towards well-rated titles among equally similar ones
                let cos = dot / (tn * cn);
                (v.id, cos * (0.9 + 0.01 * v.rating.clamp(0.0, 10.0)))
            })
        })
        .collect();
    scored.sort_by(|a, b| b.1.total_cmp(&a.1));
    scored.truncate(limit);
    scored
}

/// The VN's series: relations and relations of relations (sequels, fan discs, spin-offs).
/// They are already listed on the VN page, so "similar" shows other titles.
pub fn relations(cat: &Catalog, vid: u32) -> std::collections::HashSet<u32> {
    let mut out = std::collections::HashSet::new();
    let Ok(conn) = cat.connection() else {
        return out;
    };
    let Ok(mut st) = conn.prepare(
        "SELECT other FROM vn_relations WHERE vid = ?1 UNION SELECT vid FROM vn_relations WHERE other = ?1",
    ) else {
        return out;
    };
    let mut frontier = vec![vid];
    for _ in 0..2 {
        let mut next = Vec::new();
        for v in frontier {
            if let Ok(rows) = st.query_map([v], |r| r.get::<_, u32>(0)) {
                for r in rows.flatten() {
                    if r != vid && out.insert(r) {
                        next.push(r);
                    }
                }
            }
        }
        frontier = next;
    }
    out
}
