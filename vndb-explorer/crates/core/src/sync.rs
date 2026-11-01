//! Incremental updates through the public VNDB API (no full dump needed).
//!
//! New VNs and releases have ids above the newest ones in the local database, so
//! `id > max` finds everything that appeared since the last import. If the gap is large,
//! [`check`] reports `big = true` and the caller should run a full [`crate::import`] instead
//! (automatically or after asking the user).

use crate::db;
use crate::progress::{Progress, Reporter};
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

const API: &str = "https://api.vndb.org/kana";

/// When to prefer a full re-import over API sync.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Thresholds {
    pub max_new_vns: u64,
    pub max_new_releases: u64,
    /// Ratings, tags and old releases only change with a full import.
    pub max_days_since_import: f64,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            max_new_vns: 1500,
            max_new_releases: 6000,
            max_days_since_import: 90.0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub last_vn: u32,
    pub last_release: u32,
    pub new_vns: u64,
    pub new_releases: u64,
    pub dump_timestamp: String,
    pub imported_at: String,
    pub synced_at: String,
    pub days_since_import: f64,
    /// Too far behind for API sync: do a full update.
    pub big: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Applied {
    pub new_vns: usize,
    pub new_releases: usize,
    pub touched_vns: usize,
}

struct Api {
    http: reqwest::blocking::Client,
}

impl Api {
    fn new() -> Result<Self> {
        Ok(Self {
            http: crate::http::blocking_client(Some(Duration::from_secs(60)))?,
        })
    }

    fn post(&self, endpoint: &str, body: &Value) -> Result<Value> {
        for attempt in 0..6u64 {
            let r = self
                .http
                .post(format!("{API}/{endpoint}"))
                .json(body)
                .send()
                .context("VNDB API request failed")?;
            let status = r.status();
            if status.as_u16() == 429 {
                std::thread::sleep(Duration::from_secs(3 + attempt * 3));
                continue;
            }
            let text = r.text()?;
            if !status.is_success() {
                bail!("VNDB API error {status}: {}", text.trim());
            }
            return Ok(serde_json::from_str(&text)?);
        }
        bail!("VNDB API is throttling requests, try again later")
    }

    fn count(&self, endpoint: &str, filters: Value) -> Result<u64> {
        let v = self.post(
            endpoint,
            &json!({ "filters": filters, "results": 0, "count": true }),
        )?;
        Ok(v["count"].as_u64().unwrap_or(0))
    }

    /// All pages of `id > after`, sorted by id, at most `cap` items.
    #[allow(clippy::too_many_arguments)]
    fn newer(
        &self,
        endpoint: &str,
        prefix: &str,
        after: u32,
        fields: &str,
        cap: usize,
        rep: &Reporter,
        (stage, step): (&'static str, u32),
    ) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut page = 1;
        let mut total = None;
        rep.progress(Progress {
            stage,
            step,
            steps: 4,
            done: 0,
            total: None,
            unit: "items",
        });
        loop {
            let v = self.post(
                endpoint,
                &json!({ "filters": ["id", ">", format!("{prefix}{after}")], "fields": fields, "sort": "id", "results": 100, "page": page, "count": page == 1 }),
            )?;
            if let Some(c) = v["count"].as_u64() {
                total = Some(c.min(cap as u64));
            }
            let items = v["results"].as_array().cloned().unwrap_or_default();
            out.extend(items);
            rep.progress(Progress {
                stage,
                step,
                steps: 4,
                done: out.len() as u64,
                total,
                unit: "items",
            });
            rep.log(&format!("Fetched {} new {endpoint} entries", out.len()));
            if !v["more"].as_bool().unwrap_or(false) || out.len() >= cap {
                break;
            }
            page += 1;
        }
        Ok(out)
    }
}

fn meta(c: &Connection, key: &str) -> String {
    c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn max_ids(c: &Connection) -> Result<(u32, u32)> {
    let v: u32 = c.query_row("SELECT COALESCE(MAX(id), 0) FROM vn", [], |r| r.get(0))?;
    let r: u32 = c.query_row("SELECT COALESCE(MAX(id), 0) FROM releases", [], |r| {
        r.get(0)
    })?;
    Ok((v, r))
}

fn days_since(c: &Connection, ts: &str) -> f64 {
    if ts.is_empty() {
        return f64::INFINITY;
    }
    c.query_row("SELECT julianday('now') - julianday(?1)", [ts], |r| {
        r.get::<_, Option<f64>>(0)
    })
    .ok()
    .flatten()
    .unwrap_or(f64::INFINITY)
}

/// Compare the local database with VNDB.
pub fn check(db_path: &Path, th: &Thresholds) -> Result<Check> {
    let c = db::open_catalog(db_path)?;
    let (last_vn, last_release) = max_ids(&c)?;
    let imported_at = meta(&c, "imported_at");
    let days = days_since(&c, &imported_at);
    let api = Api::new()?;
    let new_vns = api.count("vn", json!(["id", ">", format!("v{last_vn}")]))?;
    let new_releases = api.count("release", json!(["id", ">", format!("r{last_release}")]))?;
    let reason = if new_vns > th.max_new_vns {
        Some(format!("{new_vns} new visual novels"))
    } else if new_releases > th.max_new_releases {
        Some(format!("{new_releases} new releases"))
    } else if days > th.max_days_since_import {
        Some(format!("database is {days:.0} days old"))
    } else {
        None
    };
    Ok(Check {
        last_vn,
        last_release,
        new_vns,
        new_releases,
        dump_timestamp: meta(&c, "dump_timestamp"),
        synced_at: meta(&c, "synced_at"),
        imported_at,
        days_since_import: if days.is_finite() {
            (days * 10.0).round() / 10.0
        } else {
            -1.0
        },
        big: reason.is_some(),
        reason,
    })
}

const VN_FIELDS: &str = "title,titles.lang,titles.title,titles.latin,titles.official,aliases,olang,devstatus,image.id,image.sexual,image.violence,\
length_minutes,length_votes,description,rating,votecount,tags.id,tags.rating,tags.spoiler,tags.lie,extlinks.url,extlinks.name,\
screenshots.id,screenshots.sexual,screenshots.violence,screenshots.release.id,relations.id,relations.relation,relations.relation_official";

const RELEASE_FIELDS: &str = "title,languages.lang,languages.title,languages.latin,languages.mtl,released,minage,patch,freeware,uncensored,official,\
has_ero,engine,platforms,vns.id,vns.rtype,producers.id,producers.developer,producers.publisher,producers.name,producers.original,extlinks.url,extlinks.name";

fn num(id: &Value) -> Option<u32> {
    id.as_str().and_then(crate::copy::vndbid)
}

/// "2023-01-15" -> 20230115, "2023-01" -> 20230199, "2023" -> 20239999, "TBA" -> 99999999
pub fn api_date(s: Option<&str>) -> i64 {
    let Some(s) = s else { return 0 };
    if s == "TBA" {
        return 99999999;
    }
    let p: Vec<i64> = s.split('-').filter_map(|x| x.parse().ok()).collect();
    match p.as_slice() {
        [y, m, d] => y * 10000 + m * 100 + d,
        [y, m] => y * 10000 + m * 100 + 99,
        [y] => y * 10000 + 9999,
        _ => 0,
    }
}

fn level(v: &Value) -> i64 {
    (v.as_f64().unwrap_or(0.0) * 100.0).round() as i64
}

/// Fetch and store everything newer than the local database.
pub fn apply(db_path: &Path, rep: &Reporter) -> Result<Applied> {
    let progress = |m: &str| rep.log(m);
    let mut c = db::open_catalog_rw(db_path)?;
    let (last_vn, last_release) = max_ids(&c)?;
    let api = Api::new()?;
    progress("Looking for new visual novels");
    let vns = api.newer("vn", "v", last_vn, VN_FIELDS, 5000, rep, ("sync_vn", 1))?;
    progress("Looking for new releases");
    let rels = api.newer(
        "release",
        "r",
        last_release,
        RELEASE_FIELDS,
        20000,
        rep,
        ("sync_rel", 2),
    )?;
    rep.stage("sync_save", 3, 4);

    let tx = c.transaction()?;
    let mut touched: BTreeSet<u32> = BTreeSet::new();
    let mut out = Applied::default();
    for v in &vns {
        let Some(id) = num(&v["id"]) else { continue };
        touched.insert(id);
        let titles = v["titles"].as_array().cloned().unwrap_or_default();
        let olang = v["olang"].as_str().unwrap_or("");
        let aliases: Vec<String> = v["aliases"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        tx.execute(
            "INSERT OR REPLACE INTO vn(id, olang, image, img_sexual, img_violence, votecount, rating, average, length_minutes, length_votes, length_cat,
                                       devstatus, aliases, description, released, has_ero, minage, freeware, platforms, developers)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?7,?8,?9,0,?10,?11,?12,0,0,-1,0,'','')",
            params![
                id,
                olang,
                v["image"]["id"].as_str(),
                level(&v["image"]["sexual"]),
                level(&v["image"]["violence"]),
                v["votecount"].as_u64().unwrap_or(0),
                v["rating"].as_f64().map(|r| r / 10.0),
                v["length_minutes"].as_u64(),
                v["length_votes"].as_u64().unwrap_or(0),
                v["devstatus"].as_u64().unwrap_or(0),
                aliases.join("\n"),
                v["description"].as_str().unwrap_or(""),
            ],
        )?;
        for t in [
            "vn_titles",
            "vn_tags",
            "vn_links",
            "vn_screenshots",
            "vn_relations",
        ] {
            tx.execute(&format!("DELETE FROM {t} WHERE vid = ?1"), [id])?;
        }
        for t in &titles {
            tx.execute(
                "INSERT INTO vn_titles(vid, lang, official, title, latin) VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    t["lang"].as_str(),
                    t["official"].as_bool().unwrap_or(false),
                    t["title"].as_str(),
                    t["latin"].as_str()
                ],
            )?;
        }
        for t in v["tags"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            let Some(tag) = num(&t["id"]) else { continue };
            let rating = t["rating"].as_f64().unwrap_or(0.0);
            if rating > 0.0 {
                tx.execute(
                    "INSERT INTO vn_tags(vid, tag, rating, spoiler, lie) VALUES (?1,?2,?3,?4,?5)",
                    params![
                        id,
                        tag,
                        rating,
                        t["spoiler"].as_u64().unwrap_or(0),
                        t["lie"].as_bool().unwrap_or(false)
                    ],
                )?;
            }
        }
        for l in v["extlinks"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            tx.execute(
                "INSERT INTO vn_links(vid, site, url) VALUES (?1,?2,?3)",
                params![id, l["name"].as_str(), l["url"].as_str()],
            )?;
        }
        for s in v["screenshots"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            tx.execute(
                "INSERT INTO vn_screenshots(vid, scr, rid, sexual, violence) VALUES (?1,?2,?3,?4,?5)",
                params![id, s["id"].as_str(), num(&s["release"]["id"]), level(&s["sexual"]), level(&s["violence"])],
            )?;
        }
        for r in v["relations"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            if let Some(o) = num(&r["id"]) {
                tx.execute(
                    "INSERT INTO vn_relations(vid, other, relation, official) VALUES (?1,?2,?3,?4)",
                    params![
                        id,
                        o,
                        r["relation"].as_str(),
                        r["relation_official"].as_bool().unwrap_or(false)
                    ],
                )?;
            }
        }
        out.new_vns += 1;
    }
    for r in &rels {
        let Some(id) = num(&r["id"]) else { continue };
        tx.execute(
            "INSERT OR REPLACE INTO releases(id, olang, released, minage, has_ero, patch, freeware, uncensored, official, engine)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
            params![
                id,
                r["languages"][0]["lang"].as_str().unwrap_or(""),
                api_date(r["released"].as_str()),
                r["minage"].as_i64().unwrap_or(-1),
                r["has_ero"].as_bool().unwrap_or(false),
                r["patch"].as_bool().unwrap_or(false),
                r["freeware"].as_bool().unwrap_or(false),
                r["uncensored"].as_bool().unwrap_or(false),
                r["official"].as_bool().unwrap_or(false),
                r["engine"].as_str(),
            ],
        )?;
        for t in [
            "release_titles",
            "release_vn",
            "release_platforms",
            "release_links",
            "release_producers",
        ] {
            tx.execute(&format!("DELETE FROM {t} WHERE rid = ?1"), [id])?;
        }
        for l in r["languages"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            tx.execute(
                "INSERT INTO release_titles(rid, lang, mtl, title, latin) VALUES (?1,?2,?3,?4,?5)",
                params![
                    id,
                    l["lang"].as_str(),
                    l["mtl"].as_bool().unwrap_or(false),
                    l["title"].as_str().or(r["title"].as_str()),
                    l["latin"].as_str()
                ],
            )?;
        }
        for v in r["vns"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if let Some(vid) = num(&v["id"]) {
                touched.insert(vid);
                tx.execute(
                    "INSERT INTO release_vn(rid, vid, rtype) VALUES (?1,?2,?3)",
                    params![id, vid, v["rtype"].as_str()],
                )?;
            }
        }
        for p in r["platforms"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            tx.execute(
                "INSERT INTO release_platforms(rid, platform) VALUES (?1,?2)",
                params![id, p.as_str()],
            )?;
        }
        for l in r["extlinks"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            tx.execute(
                "INSERT INTO release_links(rid, site, url) VALUES (?1,?2,?3)",
                params![id, l["name"].as_str(), l["url"].as_str()],
            )?;
        }
        for p in r["producers"]
            .as_array()
            .map(|a| a.as_slice())
            .unwrap_or(&[])
        {
            let Some(pid) = num(&p["id"]) else { continue };
            tx.execute(
                "INSERT INTO release_producers(rid, pid, developer, publisher) VALUES (?1,?2,?3,?4)",
                params![id, pid, p["developer"].as_bool().unwrap_or(false), p["publisher"].as_bool().unwrap_or(false)],
            )?;
            let (name, latin) = match p["original"].as_str() {
                Some(orig) => (orig.to_string(), p["name"].as_str().map(String::from)),
                None => (p["name"].as_str().unwrap_or("").to_string(), None),
            };
            tx.execute(
                "INSERT OR IGNORE INTO producers(id, type, lang, name, latin) VALUES (?1,'',?2,?3,?4)",
                params![pid, r["languages"][0]["lang"].as_str().unwrap_or(""), name, latin],
            )?;
        }
        out.new_releases += 1;
    }
    let touched: Vec<u32> = touched.into_iter().collect();
    progress("Updating languages and platforms");
    db::recompute_derived(&tx, Some(&touched))?;
    tx.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('synced_at', datetime('now'))",
        [],
    )?;
    tx.commit()?;
    out.touched_vns = touched.len();
    progress(&format!(
        "Added {} visual novels and {} releases",
        out.new_vns, out.new_releases
    ));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dates() {
        assert_eq!(api_date(Some("2023-01-15")), 20230115);
        assert_eq!(api_date(Some("2023-01")), 20230199);
        assert_eq!(api_date(Some("2023")), 20239999);
        assert_eq!(api_date(Some("TBA")), 99999999);
        assert_eq!(api_date(None), 0);
    }
}
