//! Import the full VNDB database dump (`vndb-db-latest.tar.zst`) into a compact SQLite file.
//!
//! Everything is streamed: download -> zstd -> tar -> line parser -> SQLite. Rows go straight
//! into the database (raw helper tables are prefixed `tmp_`), and all aggregation (tag votes,
//! per-VN languages, platforms, release dates ...) is done in SQL afterwards. Peak memory stays
//! low enough for phones.

use crate::copy::{split, Row};
use crate::db;
use crate::extlinks;
use crate::progress::{Progress, Reporter};
use anyhow::{bail, Context, Result};
use rusqlite::types::Value;
use rusqlite::{params, Connection};
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

/// Run a full import. `progress` receives human-readable status lines.
pub fn import(src: Source, db_path: &Path, rep: &Reporter) -> Result<Stats> {
    let dir = db_path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| ".".into());
    let downloaded;
    let file: &Path = match src {
        Source::Url(u) => {
            downloaded = download(u, &dir, rep)?;
            &downloaded
        }
        Source::File(p) => p,
    };
    let reader: Box<dyn Read> = Box::new(
        std::fs::File::open(file).with_context(|| format!("cannot open {}", file.display()))?,
    );
    let stats = import_reader(reader, file_len(file), db_path, rep)?;
    if matches!(src, Source::Url(_)) {
        let _ = std::fs::remove_file(file);
    }
    Ok(stats)
}

fn file_len(p: &Path) -> Option<u64> {
    std::fs::metadata(p).ok().map(|m| m.len())
}

/// Counts compressed bytes consumed by the decoder and reports the "read" stage.
struct Counting<'a, R> {
    inner: R,
    rep: &'a Reporter<'a>,
    done: u64,
    total: Option<u64>,
    last: u64,
}

impl<R: Read> Read for Counting<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.done += n as u64;
        if self.done - self.last >= 256 * 1024 || n == 0 {
            self.last = self.done;
            self.rep.progress(Progress {
                stage: "read",
                step: 2,
                steps: 8,
                done: self.done,
                total: self.total,
                unit: "bytes",
            });
        }
        Ok(n)
    }
}

/// Download the dump into `dir`, resuming after dropped connections (mobile networks, VPNs,
/// the phone going to sleep). A partial file is kept between attempts and between runs, and is
/// only resumed if it belongs to the same dump (the dated file name behind `-latest`).
pub fn download(url: &str, dir: &Path, rep: &Reporter) -> Result<std::path::PathBuf> {
    let progress = |m: &str| rep.log(m);
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let http = crate::http::blocking_client(None)?;
    // `-latest` redirects to the dated file: resolve it so a resume never mixes two dumps.
    let probe = http
        .get(url)
        .header("Range", "bytes=0-0")
        .send()
        .context("cannot reach dl.vndb.org")?;
    let real = probe.url().to_string();
    let name = real
        .rsplit('/')
        .next()
        .filter(|n| n.ends_with(".tar.zst"))
        .unwrap_or("vndb-db.tar.zst")
        .to_string();
    let total: Option<u64> = probe
        .headers()
        .get("content-range")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.rsplit('/').next())
        .and_then(|v| v.parse().ok());
    drop(probe);
    // forget partial files of older dumps
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.ends_with(".tar.zst.part") && n != format!("{name}.part") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    let part = dir.join(format!("{name}.part"));
    let done = dir.join(&name);
    if done.exists()
        && total
            .map(|t| {
                std::fs::metadata(&done)
                    .map(|m| m.len() == t)
                    .unwrap_or(false)
            })
            .unwrap_or(false)
    {
        return Ok(done);
    }
    let mb = |b: u64| b as f64 / 1_048_576.0;
    let mut attempt = 0u32;
    loop {
        let have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if let Some(t) = total {
            if have >= t {
                break;
            }
        }
        progress(&if have > 0 {
            format!("Resuming download at {:.0} MB", mb(have))
        } else {
            format!("Downloading {real}")
        });
        let result = (|| -> Result<()> {
            let mut req = http.get(&real);
            if have > 0 {
                req = req.header("Range", format!("bytes={have}-"));
            }
            let mut resp = req.send()?;
            let status = resp.status();
            let append = status.as_u16() == 206;
            if !status.is_success() {
                bail!("HTTP {status}");
            }
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .write(true)
                .append(append)
                .truncate(!append)
                .open(&part)?;
            let mut got = if append { have } else { 0 };
            let mut last_report = got;
            let mut last_tick = got;
            let tick = |done: u64| {
                rep.progress(Progress {
                    stage: "download",
                    step: 1,
                    steps: 8,
                    done,
                    total,
                    unit: "bytes",
                })
            };
            tick(got);
            let mut buf = vec![0u8; 256 * 1024];
            loop {
                let n = resp.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                f.write_all(&buf[..n])?;
                got += n as u64;
                if got - last_tick >= 256 * 1024 {
                    last_tick = got;
                    tick(got);
                }
                if got - last_report >= 16 * 1_048_576 {
                    last_report = got;
                    match total {
                        Some(t) => progress(&format!(
                            "Downloaded {:.0} of {:.0} MB ({:.0}%)",
                            mb(got),
                            mb(t),
                            got as f64 * 100.0 / t as f64
                        )),
                        None => progress(&format!("Downloaded {:.0} MB", mb(got))),
                    }
                }
            }
            f.flush()?;
            if let Some(t) = total {
                if got < t {
                    bail!("connection closed at {:.0} of {:.0} MB", mb(got), mb(t));
                }
            }
            Ok(())
        })();
        match result {
            Ok(()) => break,
            Err(e) => {
                attempt += 1;
                if attempt >= 10 {
                    return Err(e.context("download failed after 10 attempts; press the button again to continue where it stopped"));
                }
                let wait = (2u64 << attempt.min(4)).min(30);
                progress(&format!("Connection lost ({e:#}), retrying in {wait} s"));
                std::thread::sleep(std::time::Duration::from_secs(wait));
            }
        }
    }
    std::fs::rename(&part, &done)?;
    progress("Download complete");
    Ok(done)
}

/// Import from any reader of a `.tar.zst` dump (`len` = compressed size, for progress).
pub fn import_reader<R: Read>(
    reader: R,
    len: Option<u64>,
    db_path: &Path,
    rep: &Reporter,
) -> Result<Stats> {
    let progress = |m: &str| rep.log(m);
    if let Some(d) = db_path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = db_path.with_extension("importing");
    let _ = std::fs::remove_file(&tmp);
    let stats = {
        let mut conn = Connection::open(&tmp)?;
        db::create_schema(&conn)?;
        conn.execute_batch(TMP_SCHEMA)?;
        let tx = conn.transaction()?;
        let counting = Counting {
            inner: reader,
            rep,
            done: 0,
            total: len,
            last: 0,
        };
        let ts = read_archive(counting, &tx, &progress)?;
        finish(&tx, &ts, rep)?;
        let stats = stats(&tx, ts)?;
        tx.commit()?;
        progress("Compacting database");
        rep.stage("compact", 7, 8);
        conn.execute_batch("PRAGMA optimize; VACUUM;")?;
        stats
    };
    std::fs::rename(&tmp, db_path)?;
    progress(&format!(
        "Done: {} visual novels, {} releases, {} tags",
        stats.vns, stats.releases, stats.tags
    ));
    Ok(stats)
}

const TMP_SCHEMA: &str = "
    CREATE TABLE tmp_tag_votes(tag INTEGER, vid INTEGER, uid INTEGER, vote INTEGER, spoiler INTEGER, lie INTEGER);
    CREATE TABLE tmp_ignored(uid INTEGER PRIMARY KEY);
    CREATE TABLE tmp_images(id TEXT PRIMARY KEY, sexual INTEGER, violence INTEGER);
    CREATE TABLE tmp_extlinks(id INTEGER PRIMARY KEY, site TEXT, value TEXT);
    CREATE TABLE tmp_links(kind TEXT, owner INTEGER, link INTEGER);
";

fn lines<R: Read>(r: R, mut f: impl FnMut(Row) -> Result<()>) -> Result<()> {
    let mut br = BufReader::with_capacity(1 << 20, r);
    let mut buf = String::new();
    loop {
        buf.clear();
        if br.read_line(&mut buf)? == 0 {
            break;
        }
        let line = buf.trim_end_matches(['\n', '\r']);
        if !line.is_empty() {
            f(Row(split(line)))?;
        }
    }
    Ok(())
}

/// Stream one table into `sql`; `map` turns a dump row into statement parameters (None = skip).
fn load<R: Read>(
    r: R,
    c: &Connection,
    sql: &str,
    map: impl Fn(&Row) -> Option<Vec<Value>>,
) -> Result<()> {
    let mut st = c.prepare(sql)?;
    lines(r, |row| {
        if let Some(vals) = map(&row) {
            st.execute(rusqlite::params_from_iter(vals))?;
        }
        Ok(())
    })
}

fn int(v: Option<i64>) -> Value {
    v.map(Value::Integer).unwrap_or(Value::Null)
}
fn text(v: Option<&str>) -> Value {
    v.map(|s| Value::Text(s.to_string())).unwrap_or(Value::Null)
}
fn s(v: &str) -> Value {
    Value::Text(v.to_string())
}
fn b(v: bool) -> Value {
    Value::Integer(v as i64)
}
fn id(r: &Row, i: usize) -> Option<Value> {
    r.id(i).map(|x| Value::Integer(x as i64))
}

fn link(kind: &'static str) -> impl Fn(&Row) -> Option<Vec<Value>> {
    move |r: &Row| Some(vec![s(kind), id(r, 0)?, int(r.i(1))])
}

fn read_archive<R: Read>(reader: R, c: &Connection, progress: &dyn Fn(&str)) -> Result<String> {
    let dec = zstd::stream::read::Decoder::new(reader)?;
    let mut ar = tar::Archive::new(dec);
    let mut timestamp = String::new();
    let mut seen_vn = false;
    for entry in ar.entries()? {
        let mut e = entry?;
        let name = e.path()?.to_string_lossy().to_string();
        let table = name.strip_prefix("db/").unwrap_or(&name).to_string();
        match table.as_str() {
            "TIMESTAMP" => {
                e.read_to_string(&mut timestamp)?;
                timestamp = timestamp.trim().to_string();
            }
            "tags" => load(
                &mut e,
                c,
                "INSERT INTO tags(id, cat, defaultspoil, searchable, applicable, name, aliases, description) VALUES (?1,?2,?3,?4,?5,?6,?7,?8)",
                |r| Some(vec![id(r, 0)?, s(&r.string(1)), int(r.i(2).or(Some(0))), b(r.b(3)), b(r.b(4)), s(&r.string(5)), s(&r.string(6)), s(&r.string(7))]),
            )?,
            "tags_parents" => load(&mut e, c, "INSERT INTO tag_parents(id, parent, main) VALUES (?1,?2,?3)", |r| {
                Some(vec![id(r, 0)?, id(r, 1)?, b(r.b(2))])
            })?,
            "tags_vn" => {
                progress("Reading tag votes");
                // date tag vid uid vote spoiler ignore lie notes
                load(&mut e, c, "INSERT INTO tmp_tag_votes VALUES (?1,?2,?3,?4,?5,?6)", |r| {
                    if r.b(6) {
                        return None;
                    }
                    Some(vec![id(r, 1)?, id(r, 2)?, int(r.id(3).map(i64::from)), int(r.i(4)), int(r.i(5)), int(r.ob(7).map(i64::from))])
                })?
            }
            "users" => load(&mut e, c, "INSERT OR IGNORE INTO tmp_ignored VALUES (?1)", |r| if r.b(1) { Some(vec![id(r, 0)?]) } else { None })?,
            "images" => load(&mut e, c, "INSERT OR REPLACE INTO tmp_images VALUES (?1,?2,?3)", |r| {
                let i = r.string(0);
                if !(i.starts_with("cv") || i.starts_with("sf")) {
                    return None;
                }
                let sex = r.f(4).unwrap_or(0.0).round().clamp(0.0, 200.0) as i64;
                let vio = r.f(6).unwrap_or(0.0).round().clamp(0.0, 200.0) as i64;
                Some(vec![s(&i), Value::Integer(sex), Value::Integer(vio)])
            })?,
            "extlinks" => load(&mut e, c, "INSERT OR REPLACE INTO tmp_extlinks VALUES (?1,?2,?3)", |r| {
                Some(vec![int(r.i(0)), s(&r.string(1)), s(&r.string(2))])
            })?,
            "releases_extlinks" => load(&mut e, c, "INSERT INTO tmp_links VALUES (?1,?2,?3)", link("r"))?,
            "producers_extlinks" => load(&mut e, c, "INSERT INTO tmp_links VALUES (?1,?2,?3)", link("p"))?,
            "vn_extlinks" => load(&mut e, c, "INSERT INTO tmp_links VALUES (?1,?2,?3)", link("v"))?,
            "producers" => load(&mut e, c, "INSERT INTO producers(id, type, lang, name, latin) VALUES (?1,?2,?3,?4,?5)", |r| {
                Some(vec![id(r, 0)?, s(&r.string(1)), s(&r.string(2)), s(&r.string(3)), text(r.opt(4).as_deref())])
            })?,
            "releases" => {
                progress("Reading releases");
                // id gtin olang released voiced reso_x reso_y minage ... has_ero(17) patch(18) freeware(19) uncensored(20) official(21) catalog notes engine(24)
                load(
                    &mut e,
                    c,
                    "INSERT INTO releases(id, olang, released, minage, has_ero, patch, freeware, uncensored, official, engine) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                    |r| {
                        Some(vec![
                            id(r, 0)?,
                            s(&r.string(2)),
                            int(r.i(3).or(Some(0))),
                            int(r.i(7).or(Some(-1))),
                            b(r.b(17)),
                            b(r.b(18)),
                            b(r.b(19)),
                            b(r.b(20)),
                            b(r.b(21)),
                            text(r.opt(24).as_deref()),
                        ])
                    },
                )?
            }
            "releases_titles" => load(&mut e, c, "INSERT INTO release_titles(rid, lang, mtl, title, latin) VALUES (?1,?2,?3,?4,?5)", |r| {
                Some(vec![id(r, 0)?, s(&r.string(1)), b(r.b(2)), text(r.opt(3).as_deref()), text(r.opt(4).as_deref())])
            })?,
            "releases_vn" => load(&mut e, c, "INSERT INTO release_vn(rid, vid, rtype) VALUES (?1,?2,?3)", |r| {
                Some(vec![id(r, 0)?, id(r, 1)?, s(&r.string(2))])
            })?,
            "releases_platforms" => load(&mut e, c, "INSERT INTO release_platforms(rid, platform) VALUES (?1,?2)", |r| {
                Some(vec![id(r, 0)?, s(&r.string(1))])
            })?,
            "releases_producers" => load(&mut e, c, "INSERT INTO release_producers(rid, pid, developer, publisher) VALUES (?1,?2,?3,?4)", |r| {
                Some(vec![id(r, 0)?, id(r, 1)?, b(r.b(2)), b(r.b(3))])
            })?,
            "vn" => {
                progress("Reading visual novels");
                seen_vn = true;
                // id image c_image olang c_votecount c_rating c_average c_length c_lengthnum length devstatus alias description
                load(
                    &mut e,
                    c,
                    "INSERT INTO vn(id, olang, image, img_sexual, img_violence, votecount, rating, average, length_minutes, length_votes, length_cat,
                                    devstatus, aliases, description, released, has_ero, minage, freeware, platforms, developers)
                     VALUES (?1,?2,?3,0,0,?4,?5,?6,?7,?8,?9,?10,?11,?12,0,0,-1,0,'','')",
                    |r| {
                        Some(vec![
                            id(r, 0)?,
                            s(&r.string(3)),
                            text(r.opt(2).or_else(|| r.opt(1)).as_deref()),
                            int(r.i(4).or(Some(0))),
                            r.i(5).map(|x| Value::Real(x as f64 / 100.0)).unwrap_or(Value::Null),
                            r.i(6).map(|x| Value::Real(x as f64 / 100.0)).unwrap_or(Value::Null),
                            int(r.i(7)),
                            int(r.i(8).or(Some(0))),
                            int(r.i(9).or(Some(0))),
                            int(r.i(10).or(Some(0))),
                            s(&r.string(11)),
                            s(&r.string(12)),
                        ])
                    },
                )?
            }
            "vn_titles" => load(&mut e, c, "INSERT INTO vn_titles(vid, lang, official, title, latin) VALUES (?1,?2,?3,?4,?5)", |r| {
                Some(vec![id(r, 0)?, s(&r.string(1)), b(r.b(2)), s(&r.string(3)), text(r.opt(4).as_deref())])
            })?,
            "vn_screenshots" => load(&mut e, c, "INSERT INTO vn_screenshots(vid, scr, rid, sexual, violence) VALUES (?1,?2,?3,0,0)", |r| {
                Some(vec![id(r, 0)?, s(&r.string(1)), int(r.id(2).map(i64::from))])
            })?,
            "vn_relations" => load(&mut e, c, "INSERT INTO vn_relations(vid, other, relation, official) VALUES (?1,?2,?3,?4)", |r| {
                Some(vec![id(r, 0)?, id(r, 1)?, s(&r.string(2)), b(r.b(3))])
            })?,
            _ => {} // not needed: streamed past
        }
    }
    if !seen_vn {
        bail!("the archive does not look like a VNDB database dump");
    }
    Ok(timestamp)
}

fn finish(c: &Connection, timestamp: &str, rep: &Reporter) -> Result<()> {
    let progress = |m: &str| rep.log(m);
    c.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('dump_timestamp', ?1)",
        [timestamp],
    )?;
    c.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('imported_at', datetime('now'))",
        [],
    )?;
    c.execute(
        "INSERT OR REPLACE INTO meta(key, value) VALUES ('synced_at', datetime('now'))",
        [],
    )?;

    progress("Aggregating tag votes");
    rep.stage("tags", 3, 8);
    c.execute_batch(
        "INSERT INTO vn_tags(vid, tag, rating, spoiler, lie)
         SELECT tv.vid, tv.tag, ROUND(AVG(tv.vote), 2),
                CASE WHEN COUNT(tv.spoiler) = 0 THEN COALESCE((SELECT defaultspoil FROM tags t WHERE t.id = tv.tag), 0)
                     WHEN AVG(tv.spoiler) > 1.3 THEN 2 WHEN AVG(tv.spoiler) > 0.4 THEN 1 ELSE 0 END,
                COALESCE(SUM(tv.lie = 1), 0) > 0 AND COALESCE(SUM(tv.lie = 1), 0) >= COALESCE(SUM(tv.lie = 0), 0)
         FROM tmp_tag_votes tv
         WHERE tv.uid IS NULL OR tv.uid NOT IN (SELECT uid FROM tmp_ignored)
         GROUP BY tv.vid, tv.tag
         HAVING AVG(tv.vote) > 0;",
    )?;

    progress("Resolving links and images");
    rep.stage("links", 4, 8);
    {
        let mut ins_r = c.prepare("INSERT INTO release_links(rid, site, url) VALUES (?1,?2,?3)")?;
        let mut ins_v = c.prepare("INSERT INTO vn_links(vid, site, url) VALUES (?1,?2,?3)")?;
        let mut upd_p =
            c.prepare("UPDATE producers SET website = ?2 WHERE id = ?1 AND website IS NULL")?;
        let mut st = c.prepare("SELECT l.kind, l.owner, e.site, e.value FROM tmp_links l JOIN tmp_extlinks e ON e.id = l.link")?;
        let mut rows = st.query([])?;
        while let Some(r) = rows.next()? {
            let kind: String = r.get(0)?;
            let owner: i64 = r.get(1)?;
            let site: String = r.get(2)?;
            let value: String = r.get(3)?;
            let Some(url) = extlinks::url(&site, &value) else {
                continue;
            };
            match kind.as_str() {
                "r" => {
                    ins_r.execute(params![owner, site, url])?;
                }
                "v" => {
                    ins_v.execute(params![owner, site, url])?;
                }
                "p" if site == "website" => {
                    upd_p.execute(params![owner, url])?;
                }
                _ => {}
            }
        }
    }
    c.execute_batch(
        "UPDATE vn SET img_sexual = i.sexual, img_violence = i.violence FROM tmp_images i WHERE i.id = vn.image;
         UPDATE vn_screenshots SET sexual = i.sexual, violence = i.violence FROM tmp_images i WHERE i.id = vn_screenshots.scr;
         DROP TABLE tmp_tag_votes; DROP TABLE tmp_ignored; DROP TABLE tmp_images; DROP TABLE tmp_extlinks; DROP TABLE tmp_links;",
    )?;

    progress("Building indexes");
    rep.stage("indexes", 5, 8);
    db::create_indexes(c)?;
    progress("Computing languages, platforms and dates");
    rep.stage("derived", 6, 8);
    db::recompute_derived(c, None)?;
    Ok(())
}

fn stats(c: &Connection, ts: String) -> Result<Stats> {
    let n =
        |sql: &str| -> Result<usize> { Ok(c.query_row(sql, [], |r| r.get::<_, i64>(0))? as usize) };
    Ok(Stats {
        vns: n("SELECT COUNT(*) FROM vn")?,
        releases: n("SELECT COUNT(*) FROM releases")?,
        tags: n("SELECT COUNT(*) FROM tags")?,
        tag_links: n("SELECT COUNT(*) FROM vn_tags")?,
        producers: n("SELECT COUNT(*) FROM producers")?,
        dump_timestamp: ts,
    })
}
