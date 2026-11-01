//! Tag search with "required" (strict) and "wanted" (preferred) tags.
//!
//! Ranking: VNs that have *all* wanted tags come first, then by number of
//! matched wanted tags (descending), then alphabetically by shown title.

use crate::api::{Client, Fetched, Vn, VN_LIST_FIELDS};
use crate::lang::{display_title, LangPref};
use crate::tags::TagDb;
use anyhow::Result;
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone)]
pub struct Query {
    pub must: Vec<u32>,
    pub want: Vec<u32>,
    pub exclude: Vec<u32>,
    /// Minimum number of wanted tags a VN must have (only when `want` is non-empty).
    pub min_match: usize,
    /// Max spoiler level of tags considered (0 none, 1 minor, 2 major).
    pub spoiler: u8,
    /// Minimum tag rating (0..3) for a tag to count as present.
    pub min_level: f64,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub title: String,
    pub matched: Vec<u32>,
    pub missing: Vec<u32>,
    pub vn: Vn,
}

pub struct Outcome {
    pub hits: Vec<Hit>,
    pub total: Option<u64>,
    pub fetched: usize,
}

fn tag_filter(id: u32, q: &Query) -> Value {
    tag_pred(id, q, "=")
}

fn tag_pred(id: u32, q: &Query, op: &str) -> Value {
    json!(["tag", op, [format!("g{id}"), q.spoiler, q.min_level]])
}

pub fn build_filter(q: &Query, lang: &LangPref) -> Value {
    let mut and = vec![json!("and")];
    for id in &q.must {
        and.push(tag_filter(*id, q));
    }
    for id in &q.exclude {
        and.push(tag_pred(*id, q, "!="));
    }
    if !q.want.is_empty() && q.min_match > 0 {
        if q.want.len() == 1 {
            and.push(tag_filter(q.want[0], q));
        } else {
            let mut or = vec![json!("or")];
            or.extend(q.want.iter().map(|id| tag_filter(*id, q)));
            and.push(Value::Array(or));
        }
    }
    if let Some(t) = &q.title {
        and.push(json!(["search", "=", t]));
    }
    if let Some(l) = lang.filter() {
        and.push(l);
    }
    if and.len() == 2 {
        and.pop().unwrap()
    } else {
        Value::Array(and)
    }
}

/// Does the VN carry `tag` (or any of its sub-tags) under the query's thresholds?
pub fn vn_has(vn: &Vn, tag: u32, q: &Query, db: &TagDb) -> bool {
    vn.tags.iter().any(|t| {
        !t.lie
            && t.rating > 0.0
            && t.rating >= q.min_level
            && t.spoiler <= q.spoiler
            && t.id
                .trim_start_matches('g')
                .parse::<u32>()
                .map(|id| db.is_descendant(id, tag))
                .unwrap_or(false)
    })
}

pub fn score(vns: Vec<Vn>, q: &Query, db: &TagDb, lang: &LangPref) -> Vec<Hit> {
    let mut hits: Vec<Hit> = vns
        .into_iter()
        .map(|vn| {
            let (matched, missing): (Vec<u32>, Vec<u32>) =
                q.want.iter().partition(|w| vn_has(&vn, **w, q, db));
            Hit {
                title: display_title(&vn, lang),
                matched,
                missing,
                vn,
            }
        })
        .filter(|h| q.want.is_empty() || h.matched.len() >= q.min_match)
        .collect();
    sort_hits(&mut hits);
    hits
}

pub fn sort_hits(hits: &mut [Hit]) {
    hits.sort_by(|a, b| {
        let full_a = a.missing.is_empty();
        let full_b = b.missing.is_empty();
        full_b
            .cmp(&full_a)
            .then(b.matched.len().cmp(&a.matched.len()))
            .then_with(|| a.title.to_lowercase().cmp(&b.title.to_lowercase()))
            .then_with(|| a.vn.id.cmp(&b.vn.id))
    });
}

pub fn run(
    client: &Client,
    db: &TagDb,
    q: &Query,
    lang: &LangPref,
    fetch: usize,
) -> Result<Outcome> {
    // Most-voted first so that, if the candidate pool is capped, the best known titles are kept.
    let sort = Some(("votecount", true));
    let mut items: Vec<Vn> = Vec::new();

    // Pass 1: VNs having *every* wanted tag, so they are never lost to the fetch cap.
    if q.want.len() > 1 {
        let mut strict = q.clone();
        strict.must.extend(q.want.iter().copied());
        strict.want.clear();
        let r: Fetched<Vn> = client.query(
            "vn",
            build_filter(&strict, lang),
            VN_LIST_FIELDS,
            sort,
            fetch,
        )?;
        items.extend(r.items);
    }

    // Pass 2: partial matches.
    let r: Fetched<Vn> = client.query("vn", build_filter(q, lang), VN_LIST_FIELDS, sort, fetch)?;
    let fetched = r.items.len();
    let seen: std::collections::HashSet<String> = items.iter().map(|v| v.id.clone()).collect();
    items.extend(r.items.into_iter().filter(|v| !seen.contains(&v.id)));
    Ok(Outcome {
        hits: score(items, q, db, lang),
        total: r.total,
        fetched,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tags::tests::sample;

    fn vn(id: &str, title: &str, tags: &[(u32, f64)]) -> Vn {
        let tags: Vec<Value> = tags
            .iter()
            .map(|(t, r)| json!({"id": format!("g{t}"), "rating": r, "spoiler": 0, "lie": false}))
            .collect();
        serde_json::from_value(json!({"id": id, "title": title, "tags": tags})).unwrap()
    }

    fn q(want: Vec<u32>) -> Query {
        Query {
            must: vec![],
            want,
            exclude: vec![],
            min_match: 1,
            spoiler: 0,
            min_level: 0.0,
            title: None,
        }
    }

    #[test]
    fn ranking_full_then_count_then_alpha() {
        let db = sample();
        let lang = LangPref::new(None, true);
        // want: Drama(2, includes Tragedy 3), Comedy(4), ADV(6)
        let vns = vec![
            vn("v1", "Zeta", &[(3, 2.0), (4, 1.0)]),          // 2 of 3
            vn("v2", "Alpha", &[(4, 1.0)]),                   // 1 of 3
            vn("v3", "Beta", &[(2, 2.0), (4, 1.0), (6, 2.)]), // 3 of 3
            vn("v4", "Aardvark", &[(2, 1.0), (6, 1.0)]),      // 2 of 3
            vn("v5", "Nothing", &[(5, 1.0)]),                 // 0 -> dropped
            vn("v6", "Negative", &[(2, -1.0)]),               // voted down -> 0 -> dropped
        ];
        let hits = score(vns, &q(vec![2, 4, 6]), &db, &lang);
        let order: Vec<&str> = hits.iter().map(|h| h.title.as_str()).collect();
        assert_eq!(order, vec!["Beta", "Aardvark", "Zeta", "Alpha"]);
        assert!(hits[0].missing.is_empty());
    }

    #[test]
    fn filter_shape() {
        let lang = LangPref::new(Some("en"), false);
        let mut query = q(vec![2, 4]);
        query.must = vec![6];
        query.exclude = vec![5];
        let f = build_filter(&query, &lang);
        let s = f.to_string();
        assert!(s.starts_with("[\"and\""));
        assert!(s.contains("[\"tag\",\"!=\",[\"g5\""));
        assert!(s.contains("[\"or\",[\"tag\",\"=\",[\"g2\""));
        assert!(s.contains("[\"lang\",\"=\",\"en\"]"));
    }
}
