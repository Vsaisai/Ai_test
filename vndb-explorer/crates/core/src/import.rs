//! Import the full VNDB database dump (`vndb-db-latest.tar.zst`) into a compact SQLite file.
//!
//! The archive is streamed (download -> zstd -> tar -> line parser), only the tables we need
//! are kept, per-user tag votes are aggregated the same way VNDB does it, and the result is
//! written into a fresh SQLite database which then atomically replaces the old one.

use crate::copy::{split, Row};
use crate::db;
use crate::extlinks;
use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read};
use std::path::Path;

pub const DUMP_URL: &str = "https://dl.vndb.org/dump/vndb-db-latest.tar.zst";

pub enum Source<'a> {
    Url(&'a str),
    File(&'a Path),
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Stats {
    pub vns: usize,
    pub releases: usize,
    pub tags: usize,
    pub tag_links: usize,
    pub producers: usize,
    pub dump_timestamp: String,
}

#[derive(Default)]
struct TagVote {
    vid: u32,
    tag: u32,
    uid: u32,
    vote: i8,
    spoiler: i8, // -1 = none
    lie: i8,     // -1 = none, 0 false, 1 true
}

#[derive(Default)]
struct Raw {
    timestamp: String,
    tags: Vec<Row>,
    tags_parents: Vec<(u32, u32, bool)>,
    tag_votes: Vec<TagVote>,
    ignored_users: HashSet<u32>,
    vn: Vec<Row>,
    vn_titles: Vec<Row>,
    vn_extlinks: Vec<(u32, u32)>,
    vn_screenshots: Vec<(u32, String, Option<u32>)>,
    vn_relations: Vec<(u32, u32, String, bool)>,
    images: HashMap<String, (u16, u16)>,
    releases: Vec<Row>,
    releases_titles: Vec<Row>,
    releases_vn: Vec<(u32, u32, String)>,
    releases_platforms: Vec<(u32, String)>,
    releases_extlinks: Vec<(u32, u32)>,
    releases_producers: Vec<(u32, u32, bool, bool)>,
    extlinks: HashMap<u32, (String, String)>,
    producers: Vec<Row>,
    producers_extlinks: Vec<(u32, u32)>,
}

fn lines<R: Read>(r: R, mut f: impl FnMut(Row)) -> Result<()> {
    let mut br = BufReader::with_capacity(1 << 20, r);
    let mut buf = String::new();
    loop {
        buf.clear();
        if br.read_line(&mut buf)? == 0 {
            break;
        }
        let line = buf.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            f(Row(split(line)));
        }
    }
    Ok(())
}

fn small(v: Option<i64>) -> i8 {
    v.map(|x| x.clamp(-128, 127) as i8).unwrap_or(-1)
}

/// Run a full import. `progress` receives human-readable status lines.
pub fn import(src: Source, db_path: &Path, progress: &dyn Fn(&str)) -> Result<Stats> {
    let reader: Box<dyn Read> = match src {
        Source::Url(u) => {
            progress(&format!("Downloading {u}"));
            let resp = reqwest::blocking::Client::builder()
                .user_agent(concat!("vndb-explorer/", env!("CARGO_PKG_VERSION")))
                .timeout(None)
                .build()?
                .get(u)
                .send()
                .context("download of the VNDB dump failed")?;
            if !resp.status().is_success() {
                bail!("dump download failed: HTTP {}", resp.status());
            }
            Box::new(resp)
        }
        Source::File(p) => Box::new(
            std::fs::File::open(p).with_context(|| format!("cannot open {}", p.display()))?,
        ),
    };
    let raw = read_archive(reader, progress)?;
    progress("Writing database");
    let tmp = db_path.with_extension("importing");
    let _ = std::fs::remove_file(&tmp);
    if let Some(d) = db_path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let stats = {
        let mut conn = Connection::open(&tmp)?;
        db::create_schema(&conn)?;
        let s = write(&mut conn, raw, progress)?;
        conn.execute_batch("PRAGMA optimize; VACUUM;")?;
        s
    };
    std::fs::rename(&tmp, db_path)?;
    progress(&format!(
        "Done: {} visual novels, {} releases, {} tags",
        stats.vns, stats.releases, stats.tags
    ));
    Ok(stats)
}

fn read_archive(reader: Box<dyn Read>, progress: &dyn Fn(&str)) -> Result<Raw> {
    let dec = zstd::stream::read::Decoder::new(reader)?;
    let mut ar = tar::Archive::new(dec);
    let mut raw = Raw::default();
    for entry in ar.entries()? {
        let mut e = entry?;
        let name = e.path()?.to_string_lossy().to_string();
        let table = name.strip_prefix("db/").unwrap_or(&name).to_string();
        let r = &mut raw;
        match table.as_str() {
            "TIMESTAMP" => {
                let mut s = String::new();
                e.read_to_string(&mut s)?;
                r.timestamp = s.trim().to_string();
            }
            "tags" => lines(&mut e, |row| r.tags.push(row))?,
            "tags_parents" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.id(1)) {
                    r.tags_parents.push((a, b, x.b(2)))
                }
            })?,
            "tags_vn" => {
                progress("Reading tag votes");
                // date tag vid uid vote spoiler ignore lie notes
                lines(&mut e, |x| {
                    if x.b(6) {
                        return;
                    }
                    if let (Some(tag), Some(vid)) = (x.id(1), x.id(2)) {
                        r.tag_votes.push(TagVote {
                            vid,
                            tag,
                            uid: x.id(3).unwrap_or(0),
                            vote: small(x.i(4)),
                            spoiler: small(x.i(5)),
                            lie: x.ob(7).map(|b| b as i8).unwrap_or(-1),
                        })
                    }
                })?
            }
            "users" => lines(&mut e, |x| {
                // id ign_votes ...
                if x.b(1) {
                    if let Some(id) = x.id(0) {
                        r.ignored_users.insert(id);
                    }
                }
            })?,
            "vn" => {
                progress("Reading visual novels");
                lines(&mut e, |row| r.vn.push(row))?
            }
            "vn_titles" => lines(&mut e, |row| r.vn_titles.push(row))?,
            "vn_extlinks" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.i(1)) {
                    r.vn_extlinks.push((a, b as u32))
                }
            })?,
            "vn_screenshots" => lines(&mut e, |x| {
                if let Some(v) = x.id(0) {
                    r.vn_screenshots.push((v, x.string(1), x.id(2)))
                }
            })?,
            "vn_relations" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.id(1)) {
                    r.vn_relations.push((a, b, x.string(2), x.b(3)))
                }
            })?,
            "images" => lines(&mut e, |x| {
                let id = x.string(0);
                if id.starts_with("cv") || id.starts_with("sf") {
                    let sex = x.f(4).unwrap_or(0.0).round().clamp(0.0, 200.0) as u16;
                    let vio = x.f(6).unwrap_or(0.0).round().clamp(0.0, 200.0) as u16;
                    r.images.insert(id, (sex, vio));
                }
            })?,
            "releases" => {
                progress("Reading releases");
                lines(&mut e, |row| r.releases.push(row))?
            }
            "releases_titles" => lines(&mut e, |row| r.releases_titles.push(row))?,
            "releases_vn" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.id(1)) {
                    r.releases_vn.push((a, b, x.string(2)))
                }
            })?,
            "releases_platforms" => lines(&mut e, |x| {
                if let Some(a) = x.id(0) {
                    r.releases_platforms.push((a, x.string(1)))
                }
            })?,
            "releases_extlinks" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.i(1)) {
                    r.releases_extlinks.push((a, b as u32))
                }
            })?,
            "releases_producers" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.id(1)) {
                    r.releases_producers.push((a, b, x.b(2), x.b(3)))
                }
            })?,
            "extlinks" => lines(&mut e, |x| {
                if let Some(id) = x.i(0) {
                    r.extlinks.insert(id as u32, (x.string(1), x.string(2)));
                }
            })?,
            "producers" => lines(&mut e, |row| r.producers.push(row))?,
            "producers_extlinks" => lines(&mut e, |x| {
                if let (Some(a), Some(b)) = (x.id(0), x.i(1)) {
                    r.producers_extlinks.push((a, b as u32))
                }
            })?,
            _ => {} // skipped (streamed past)
        }
    }
    if raw.vn.is_empty() || raw.tags.is_empty() {
        bail!("the archive does not look like a VNDB database dump");
    }
    Ok(raw)
}

/// "20031017" -> 20031017; 0 = unknown; 99999999 = TBA
fn date(s: Option<i64>) -> i64 {
    s.unwrap_or(0)
}

fn write(conn: &mut Connection, raw: Raw, progress: &dyn Fn(&str)) -> Result<Stats> {
    let tx = conn.transaction()?;
    let mut stats = Stats {
        dump_timestamp: raw.timestamp.clone(),
        ..Default::default()
    };
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('dump_timestamp', ?1)",
        [&raw.timestamp],
    )?;
    tx.execute(
        "INSERT INTO meta(key, value) VALUES ('imported_at', datetime('now'))",
        [],
    )?;

    // ---- tags ----
    let mut defaultspoil: HashMap<u32, i64> = HashMap::new();
    {
        let mut st = tx.prepare(
            "INSERT INTO tags(id, cat, defaultspoil, searchable, applicable, name, aliases, description) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
        )?;
        for t in &raw.tags {
            let Some(id) = t.id(0) else { continue };
            defaultspoil.insert(id, t.i(2).unwrap_or(0));
            st.execute(params![
                id,
                t.string(1),
                t.i(2).unwrap_or(0),
                t.b(3),
                t.b(4),
                t.string(5),
                t.string(6),
                t.string(7)
            ])?;
            stats.tags += 1;
        }
        let mut st = tx.prepare("INSERT INTO tag_parents(id, parent, main) VALUES (?1,?2,?3)")?;
        for (a, b, m) in &raw.tags_parents {
            st.execute(params![a, b, m])?;
        }
    }

    // ---- tag votes -> aggregated VN tags ----
    progress("Aggregating tag votes");
    #[derive(Default)]
    struct Agg {
        sum: i64,
        n: i64,
        sp_sum: i64,
        sp_n: i64,
        lie_t: i64,
        lie_f: i64,
    }
    let mut agg: HashMap<(u32, u32), Agg> = HashMap::new();
    for v in &raw.tag_votes {
        if raw.ignored_users.contains(&v.uid) {
            continue;
        }
        let a = agg.entry((v.vid, v.tag)).or_default();
        a.sum += v.vote as i64;
        a.n += 1;
        if v.spoiler >= 0 {
            a.sp_sum += v.spoiler as i64;
            a.sp_n += 1;
        }
        match v.lie {
            1 => a.lie_t += 1,
            0 => a.lie_f += 1,
            _ => {}
        }
    }
    {
        let mut st = tx.prepare(
            "INSERT INTO vn_tags(vid, tag, rating, spoiler, lie) VALUES (?1,?2,?3,?4,?5)",
        )?;
        for ((vid, tag), a) in &agg {
            if a.n == 0 {
                continue;
            }
            let rating = a.sum as f64 / a.n as f64;
            if rating <= 0.0 {
                continue;
            }
            let spoiler = if a.sp_n == 0 {
                *defaultspoil.get(tag).unwrap_or(&0)
            } else {
                let avg = a.sp_sum as f64 / a.sp_n as f64;
                if avg > 1.3 {
                    2
                } else if avg > 0.4 {
                    1
                } else {
                    0
                }
            };
            let lie = a.lie_t > 0 && a.lie_t >= a.lie_f;
            st.execute(params![
                vid,
                tag,
                (rating * 100.0).round() / 100.0,
                spoiler,
                lie
            ])?;
            stats.tag_links += 1;
        }
    }

    // ---- extlinks helper ----
    let link = |id: &u32| -> Option<(String, String)> {
        let (site, value) = raw.extlinks.get(id)?;
        Some((site.clone(), extlinks::url(site, value)?))
    };

    // ---- producers ----
    {
        let mut website: HashMap<u32, String> = HashMap::new();
        for (pid, l) in &raw.producers_extlinks {
            if let Some((site, url)) = link(l) {
                if site == "website" {
                    website.entry(*pid).or_insert(url);
                }
            }
        }
        let mut st = tx.prepare("INSERT INTO producers(id, type, lang, name, latin, website) VALUES (?1,?2,?3,?4,?5,?6)")?;
        for p in &raw.producers {
            let Some(id) = p.id(0) else { continue };
            st.execute(params![
                id,
                p.string(1),
                p.string(2),
                p.string(3),
                p.opt(4),
                website.get(&id)
            ])?;
            stats.producers += 1;
        }
    }

    // ---- releases ----
    progress("Writing releases");
    let mut rel_info: HashMap<u32, (i64, bool, bool, bool, bool, i64)> = HashMap::new(); // released, official, patch, freeware, has_ero, minage
    {
        // releases: id gtin olang released voiced reso_x reso_y minage ani_story ani_ero ... has_ero(17) patch(18) freeware(19) uncensored(20) official(21) catalog notes engine
        let mut st = tx.prepare(
            "INSERT INTO releases(id, olang, released, minage, has_ero, patch, freeware, uncensored, official, engine) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
        )?;
        for r in &raw.releases {
            let Some(id) = r.id(0) else { continue };
            let released = date(r.i(3));
            let minage = r.i(7).unwrap_or(-1);
            rel_info.insert(id, (released, r.b(21), r.b(18), r.b(19), r.b(17), minage));
            st.execute(params![
                id,
                r.string(2),
                released,
                minage,
                r.b(17),
                r.b(18),
                r.b(19),
                r.b(20),
                r.b(21),
                r.opt(24)
            ])?;
            stats.releases += 1;
        }
        let mut st = tx.prepare(
            "INSERT INTO release_titles(rid, lang, mtl, title, latin) VALUES (?1,?2,?3,?4,?5)",
        )?;
        for t in &raw.releases_titles {
            let Some(id) = t.id(0) else { continue };
            st.execute(params![id, t.string(1), t.b(2), t.opt(3), t.opt(4)])?;
        }
        let mut st = tx.prepare("INSERT INTO release_vn(rid, vid, rtype) VALUES (?1,?2,?3)")?;
        for (r, v, t) in &raw.releases_vn {
            st.execute(params![r, v, t])?;
        }
        let mut st = tx.prepare("INSERT INTO release_platforms(rid, platform) VALUES (?1,?2)")?;
        for (r, p) in &raw.releases_platforms {
            st.execute(params![r, p])?;
        }
        let mut st = tx.prepare("INSERT INTO release_links(rid, site, url) VALUES (?1,?2,?3)")?;
        for (r, l) in &raw.releases_extlinks {
            if let Some((site, url)) = link(l) {
                st.execute(params![r, site, url])?;
            }
        }
        let mut st = tx.prepare(
            "INSERT INTO release_producers(rid, pid, developer, publisher) VALUES (?1,?2,?3,?4)",
        )?;
        for (r, p, d, pb) in &raw.releases_producers {
            st.execute(params![r, p, d, pb])?;
        }
    }

    // ---- per-VN derived data from releases ----
    // languages: lang -> best availability (0 original/official, 1 fan, 2 mtl only)
    let mut rel_langs: HashMap<u32, Vec<(String, bool)>> = HashMap::new();
    for t in &raw.releases_titles {
        if let Some(id) = t.id(0) {
            rel_langs.entry(id).or_default().push((t.string(1), t.b(2)));
        }
    }
    let mut rel_plat: HashMap<u32, Vec<String>> = HashMap::new();
    for (r, p) in &raw.releases_platforms {
        rel_plat.entry(*r).or_default().push(p.clone());
    }
    struct VnDerived {
        langs: HashMap<String, u8>,
        platforms: HashSet<String>,
        released: i64,
        has_ero: bool,
        minage: i64,
        freeware: bool,
        developers: Vec<u32>,
    }
    let mut derived: HashMap<u32, VnDerived> = HashMap::new();
    let mut rel_devs: HashMap<u32, Vec<u32>> = HashMap::new();
    for (r, p, dev, _) in &raw.releases_producers {
        if *dev {
            rel_devs.entry(*r).or_default().push(*p);
        }
    }
    for (rid, vid, rtype) in &raw.releases_vn {
        let Some(&(released, official, patch, freeware, has_ero, minage)) = rel_info.get(rid)
        else {
            continue;
        };
        let d = derived.entry(*vid).or_insert_with(|| VnDerived {
            langs: HashMap::new(),
            platforms: HashSet::new(),
            released: 0,
            has_ero: false,
            minage: -1,
            freeware: false,
            developers: vec![],
        });
        for (lang, mtl) in rel_langs.get(rid).map(|v| v.as_slice()).unwrap_or(&[]) {
            let kind = if *mtl {
                2
            } else if official {
                0
            } else {
                1
            };
            let e = d.langs.entry(lang.clone()).or_insert(kind);
            *e = (*e).min(kind);
        }
        if !patch {
            for p in rel_plat.get(rid).map(|v| v.as_slice()).unwrap_or(&[]) {
                d.platforms.insert(p.clone());
            }
        }
        if released > 0
            && released < 99999999
            && (d.released == 0 || released < d.released)
            && rtype != "trial"
        {
            d.released = released;
        }
        d.has_ero |= has_ero;
        d.minage = d.minage.max(minage);
        d.freeware |= freeware && official && !patch && rtype == "complete";
        if official {
            for p in rel_devs.get(rid).map(|v| v.as_slice()).unwrap_or(&[]) {
                if !d.developers.contains(p) {
                    d.developers.push(*p);
                }
            }
        }
    }

    // ---- VNs ----
    progress("Writing visual novels");
    {
        // vn: id image c_image olang c_votecount c_rating c_average c_length c_lengthnum length devstatus alias description
        let mut st = tx.prepare(
            "INSERT INTO vn(id, olang, image, img_sexual, img_violence, votecount, rating, average, length_minutes, length_votes, length_cat,
                            devstatus, aliases, description, released, has_ero, minage, freeware, platforms, developers)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20)",
        )?;
        let mut sl = tx.prepare("INSERT INTO vn_langs(vid, lang, kind) VALUES (?1,?2,?3)")?;
        for v in &raw.vn {
            let Some(id) = v.id(0) else { continue };
            let image = v.opt(2).or_else(|| v.opt(1));
            let (sex, vio) = image
                .as_ref()
                .and_then(|i| raw.images.get(i).copied())
                .unwrap_or((0, 0));
            let d = derived.get(&id);
            let mut plats: Vec<&String> =
                d.map(|d| d.platforms.iter().collect()).unwrap_or_default();
            plats.sort();
            let devs = d
                .map(|d| {
                    d.developers
                        .iter()
                        .map(|p| p.to_string())
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            st.execute(params![
                id,
                v.string(3),
                image,
                sex,
                vio,
                v.i(4).unwrap_or(0),
                v.i(5).map(|r| r as f64 / 100.0),
                v.i(6).map(|r| r as f64 / 100.0),
                v.i(7),
                v.i(8).unwrap_or(0),
                v.i(9).unwrap_or(0),
                v.i(10).unwrap_or(0),
                v.string(11),
                v.string(12),
                d.map(|d| d.released).unwrap_or(0),
                d.map(|d| d.has_ero).unwrap_or(false),
                d.map(|d| d.minage).unwrap_or(-1),
                d.map(|d| d.freeware).unwrap_or(false),
                plats
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
                devs,
            ])?;
            let mut langs: HashMap<String, u8> = d.map(|d| d.langs.clone()).unwrap_or_default();
            langs.entry(v.string(3)).or_insert(0);
            for (l, k) in langs {
                if !l.is_empty() {
                    sl.execute(params![id, l, k])?;
                }
            }
            stats.vns += 1;
        }
        let mut st = tx.prepare(
            "INSERT INTO vn_titles(vid, lang, official, title, latin) VALUES (?1,?2,?3,?4,?5)",
        )?;
        for t in &raw.vn_titles {
            let Some(id) = t.id(0) else { continue };
            st.execute(params![id, t.string(1), t.b(2), t.string(3), t.opt(4)])?;
        }
        let mut st = tx.prepare("INSERT INTO vn_links(vid, site, url) VALUES (?1,?2,?3)")?;
        for (v, l) in &raw.vn_extlinks {
            if let Some((site, url)) = link(l) {
                st.execute(params![v, site, url])?;
            }
        }
        let mut st = tx.prepare(
            "INSERT INTO vn_screenshots(vid, scr, rid, sexual, violence) VALUES (?1,?2,?3,?4,?5)",
        )?;
        for (v, scr, rid) in &raw.vn_screenshots {
            let (s, vi) = raw.images.get(scr).copied().unwrap_or((0, 0));
            st.execute(params![v, scr, rid, s, vi])?;
        }
        let mut st = tx.prepare(
            "INSERT INTO vn_relations(vid, other, relation, official) VALUES (?1,?2,?3,?4)",
        )?;
        for (a, b, rel, off) in &raw.vn_relations {
            st.execute(params![a, b, rel, off])?;
        }
    }
    progress("Building indexes");
    db::create_indexes(&tx)?;
    tx.commit()?;
    Ok(stats)
}
