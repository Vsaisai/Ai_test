//! "Where to get it": free downloads / fan patches, stores, developer sites,
//! and fallback search links when VNDB knows of nothing.

use crate::api::{Client, Release, Vn};
use crate::lang::LangPref;
use anyhow::Result;
use serde::Serialize;
use std::collections::HashSet;

/// Sites where (free) games are downloaded directly.
const DOWNLOAD_SITES: &[&str] = &[
    "itch", "gamejolt", "freem", "freegame", "novelgam", "booth", "patreonp",
];

/// Shops (digital or physical).
const STORES: &[&str] = &[
    "steam",
    "gog",
    "itch",
    "jastusa",
    "mg",
    "denpa",
    "fakku",
    "dlsite",
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

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Official free release (freeware) download.
    Free,
    /// Shop / store page.
    Store,
    /// Developer's or publisher's own site (buy direct).
    Direct,
    /// Unofficial translation patch / fan release.
    FanPatch,
    /// Generated search link (nothing found on VNDB).
    Search,
}

#[derive(Debug, Clone, Serialize)]
pub struct Source {
    pub kind: Kind,
    pub site: String,
    pub url: String,
    pub release: Option<String>,
    pub langs: Vec<String>,
    pub mtl: bool,
    pub price: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Where {
    pub vndb: String,
    pub sources: Vec<Source>,
}

fn release_note(r: &Release) -> String {
    let mut s = r.title.clone();
    if let Some(d) = &r.released {
        s.push_str(&format!(" ({d})"));
    }
    if !r.platforms.is_empty() {
        s.push_str(&format!(" [{}]", r.platforms.join(",")));
    }
    s
}

/// Classify release links. Releases are filtered to readable languages first.
pub fn classify(vn: &Vn, releases: &[Release], lang: &LangPref) -> Vec<Source> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let readable: Vec<&Release> = releases
        .iter()
        .filter(|r| r.languages.iter().any(|l| lang.accepts(&l.lang)))
        .collect();

    for r in &readable {
        let langs: Vec<String> = lang.sorted_langs(
            &r.languages
                .iter()
                .filter(|l| lang.accepts(&l.lang))
                .map(|l| l.lang.clone())
                .collect::<Vec<_>>(),
        );
        let mtl = r
            .languages
            .iter()
            .filter(|l| lang.accepts(&l.lang))
            .all(|l| l.mtl);
        for l in &r.extlinks {
            let kind = if !r.official {
                if l.name == "website"
                    || DOWNLOAD_SITES.contains(&l.name.as_str())
                    || STORES.contains(&l.name.as_str())
                {
                    Some(Kind::FanPatch)
                } else {
                    None
                }
            } else if r.freeware
                && (l.name == "website"
                    || DOWNLOAD_SITES.contains(&l.name.as_str())
                    || STORES.contains(&l.name.as_str()))
            {
                Some(Kind::Free)
            } else if STORES.contains(&l.name.as_str()) {
                Some(Kind::Store)
            } else if l.name == "website" {
                Some(Kind::Direct)
            } else {
                None
            };
            let Some(kind) = kind else { continue };
            if !seen.insert(l.url.clone()) {
                continue;
            }
            let site = if kind == Kind::FanPatch && l.name == "website" {
                "Translator/fan site".to_string()
            } else {
                l.label.clone()
            };
            out.push(Source {
                kind,
                site,
                url: l.url.clone(),
                release: Some(release_note(r)),
                langs: langs.clone(),
                mtl,
                price: None,
            });
        }
    }
    // VN-level store links (rare, but e.g. Steam is sometimes attached to the VN itself).
    for l in &vn.extlinks {
        if STORES.contains(&l.name.as_str()) && seen.insert(l.url.clone()) {
            out.push(Source {
                kind: Kind::Store,
                site: l.label.clone(),
                url: l.url.clone(),
                release: None,
                langs: vec![],
                mtl: false,
                price: None,
            });
        }
    }
    sort_sources(&mut out, lang);
    out
}

fn sort_sources(v: &mut [Source], lang: &LangPref) {
    v.sort_by_key(|s| {
        (
            s.kind,
            s.mtl,
            s.langs.first().map(|l| lang.rank(l)).unwrap_or(500),
            site_rank(&s.url),
            s.site.to_lowercase(),
        )
    });
}

/// Within a group: itch.io / direct-download sites first, then big stores, then the rest.
fn site_rank(url: &str) -> u8 {
    let u = url.to_lowercase();
    if u.contains("itch.io") {
        0
    } else if ["gamejolt.com", "freem.jp", "booth.pm", "patreon.com"]
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

/// Search links used when VNDB lists no download or store page.
pub fn fallback(title: &str) -> Vec<Source> {
    let q = urlencoding::encode(title);
    let mk = |site: &str, url: String| Source {
        kind: Kind::Search,
        site: site.into(),
        url,
        release: None,
        langs: vec![],
        mtl: false,
        price: None,
    };
    vec![
        mk("itch.io", format!("https://itch.io/search?q={q}")),
        mk(
            "F95zone",
            format!("https://f95zone.to/search/?q={q}&t=post&c[title_only]=1"),
        ),
        mk(
            "Steam",
            format!("https://store.steampowered.com/search/?term={q}"),
        ),
        mk("GOG", format!("https://www.gog.com/en/games?query={q}")),
        mk(
            "DLsite (EN)",
            format!("https://www.dlsite.com/ecchi-eng/fsr/=/keyword/{q}"),
        ),
    ]
}

/// Full lookup: releases, developer sites, optional Steam prices, fallbacks.
pub fn lookup(
    client: &Client,
    vn: &Vn,
    title: &str,
    lang: &LangPref,
    prices: Option<&str>,
) -> Result<Where> {
    let releases = client.releases_for(&vn.id)?;
    let mut sources = classify(vn, &releases, lang);

    // Developer / publisher homepages of commercial readable releases -> "buy direct" option.
    let has_paid = sources
        .iter()
        .any(|s| matches!(s.kind, Kind::Store | Kind::Direct));
    if has_paid {
        let mut ids: Vec<String> = releases
            .iter()
            .filter(|r| {
                r.official && !r.freeware && r.languages.iter().any(|l| lang.accepts(&l.lang))
            })
            .flat_map(|r| r.producers.iter().map(|p| p.id.clone()))
            .collect();
        ids.extend(vn.developers.iter().map(|d| d.id.clone()));
        ids.sort();
        ids.dedup();
        ids.truncate(10);
        let seen: HashSet<String> = sources.iter().map(|s| s.url.clone()).collect();
        for (name, links) in client.producer_links(&ids).unwrap_or_default() {
            for l in links
                .iter()
                .filter(|l| l.name == "website" && !seen.contains(&l.url))
            {
                sources.push(Source {
                    kind: Kind::Direct,
                    site: format!("{name} (homepage)"),
                    url: l.url.clone(),
                    release: None,
                    langs: vec![],
                    mtl: false,
                    price: None,
                });
            }
        }
        sort_sources(&mut sources, lang);
    }

    if let Some(cc) = prices {
        for s in sources
            .iter_mut()
            .filter(|s| s.url.contains("store.steampowered.com/app/"))
        {
            s.price = steam_price(client, &s.url, cc);
        }
    }

    if !sources
        .iter()
        .any(|s| matches!(s.kind, Kind::Free | Kind::Store))
    {
        sources.extend(fallback(title));
    }
    Ok(Where {
        vndb: format!("https://vndb.org/{}", vn.id),
        sources,
    })
}

fn steam_price(client: &Client, url: &str, cc: &str) -> Option<String> {
    let id: String = url
        .split("/app/")
        .nth(1)?
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let api = format!("https://store.steampowered.com/api/appdetails?appids={id}&cc={cc}&filters=basic,price_overview");
    let v: serde_json::Value = client.http().get(api).send().ok()?.json().ok()?;
    let entry = v.get(&id)?;
    if !entry.get("success")?.as_bool()? {
        return Some("unavailable in region".into());
    }
    let d = entry.get("data")?;
    if d.get("is_free").and_then(|x| x.as_bool()).unwrap_or(false) {
        return Some("free".into());
    }
    match d.get("price_overview") {
        Some(po) => {
            let fin = po.get("final_formatted")?.as_str()?.to_string();
            let disc = po
                .get("discount_percent")
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            Some(if disc > 0 {
                format!("{fin} (-{disc}%)")
            } else {
                fin
            })
        }
        None => Some("not sold separately / no price".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn classifies_release_links() {
        let vn: Vn = serde_json::from_value(json!({"id":"v1","title":"T"})).unwrap();
        let rels: Vec<Release> = serde_json::from_value(json!([
            {"id":"r1","title":"EN Steam","languages":[{"lang":"en","mtl":false}],"official":true,"freeware":false,"patch":false,
             "extlinks":[{"url":"https://store.steampowered.com/app/1/","label":"Steam","name":"steam"},
                         {"url":"https://dev.example/buy","label":"Official website","name":"website"}]},
            {"id":"r2","title":"RU patch","languages":[{"lang":"ru","mtl":false}],"official":false,"freeware":true,"patch":true,
             "extlinks":[{"url":"https://fans.example/","label":"Official website","name":"website"}]},
            {"id":"r3","title":"Free demo","languages":[{"lang":"en","mtl":false}],"official":true,"freeware":true,"patch":false,
             "extlinks":[{"url":"https://x.itch.io/demo","label":"Itch.io","name":"itch"}]},
            {"id":"r4","title":"JP only","languages":[{"lang":"ja","mtl":false}],"official":true,"freeware":false,"patch":false,
             "extlinks":[{"url":"https://dlsite.jp/","label":"DLsite","name":"dlsite"}]}
        ])).unwrap();
        let s = classify(&vn, &rels, &LangPref::new(None, false));
        let kinds: Vec<(Kind, &str)> = s.iter().map(|s| (s.kind, s.url.as_str())).collect();
        assert_eq!(
            kinds,
            vec![
                (Kind::Free, "https://x.itch.io/demo"),
                (Kind::Store, "https://store.steampowered.com/app/1/"),
                (Kind::Direct, "https://dev.example/buy"),
                (Kind::FanPatch, "https://fans.example/"),
            ]
        );
    }

    #[test]
    fn fallback_has_search_links() {
        let f = fallback("Some Game");
        assert!(f
            .iter()
            .any(|s| s.url.contains("itch.io/search?q=Some%20Game")));
        assert!(f.iter().any(|s| s.site == "F95zone"));
    }
}
