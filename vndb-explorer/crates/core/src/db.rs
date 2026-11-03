//! SQLite schema for the local catalog and the separate user data file.

use anyhow::Result;
use rusqlite::Connection;
use std::path::Path;

pub fn create_schema(c: &Connection) -> Result<()> {
    c.execute_batch(
        "
        PRAGMA journal_mode = OFF;
        PRAGMA synchronous = OFF;
        CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE tags(id INTEGER PRIMARY KEY, cat TEXT, defaultspoil INTEGER, searchable INTEGER, applicable INTEGER,
                          name TEXT, aliases TEXT, description TEXT);
        CREATE TABLE tag_parents(id INTEGER, parent INTEGER, main INTEGER);
        CREATE TABLE vn(id INTEGER PRIMARY KEY, olang TEXT, image TEXT, img_sexual INTEGER, img_violence INTEGER,
                        votecount INTEGER, rating REAL, average REAL, length_minutes INTEGER, length_votes INTEGER,
                        length_cat INTEGER, devstatus INTEGER, aliases TEXT, description TEXT, released INTEGER,
                        has_ero INTEGER, minage INTEGER, freeware INTEGER, platforms TEXT, developers TEXT);
        CREATE TABLE vn_titles(vid INTEGER, lang TEXT, official INTEGER, title TEXT, latin TEXT);
        CREATE TABLE vn_langs(vid INTEGER, lang TEXT, kind INTEGER);
        CREATE TABLE vn_tags(vid INTEGER, tag INTEGER, rating REAL, spoiler INTEGER, lie INTEGER);
        CREATE TABLE vn_links(vid INTEGER, site TEXT, url TEXT);
        CREATE TABLE vn_screenshots(vid INTEGER, scr TEXT, rid INTEGER, sexual INTEGER, violence INTEGER);
        CREATE TABLE vn_relations(vid INTEGER, other INTEGER, relation TEXT, official INTEGER);
        CREATE TABLE releases(id INTEGER PRIMARY KEY, olang TEXT, released INTEGER, minage INTEGER, has_ero INTEGER,
                              patch INTEGER, freeware INTEGER, uncensored INTEGER, official INTEGER, engine TEXT);
        CREATE TABLE release_titles(rid INTEGER, lang TEXT, mtl INTEGER, title TEXT, latin TEXT);
        CREATE TABLE release_vn(rid INTEGER, vid INTEGER, rtype TEXT);
        CREATE TABLE release_platforms(rid INTEGER, platform TEXT);
        CREATE TABLE release_links(rid INTEGER, site TEXT, url TEXT);
        CREATE TABLE release_producers(rid INTEGER, pid INTEGER, developer INTEGER, publisher INTEGER);
        CREATE TABLE producers(id INTEGER PRIMARY KEY, type TEXT, lang TEXT, name TEXT, latin TEXT, website TEXT);
        ",
    )?;
    Ok(())
}

/// Indexes are created after bulk insert (faster import).
pub fn create_indexes(c: &Connection) -> Result<()> {
    c.execute_batch(
        "
        CREATE INDEX vn_titles_vid ON vn_titles(vid);
        CREATE INDEX vn_langs_vid ON vn_langs(vid);
        CREATE INDEX vn_tags_vid ON vn_tags(vid);
        CREATE INDEX vn_links_vid ON vn_links(vid);
        CREATE INDEX vn_screenshots_vid ON vn_screenshots(vid);
        CREATE INDEX vn_relations_vid ON vn_relations(vid);
        CREATE INDEX release_vn_vid ON release_vn(vid);
        CREATE INDEX release_vn_rid ON release_vn(rid);
        CREATE INDEX release_titles_rid ON release_titles(rid);
        CREATE INDEX release_platforms_rid ON release_platforms(rid);
        CREATE INDEX release_links_rid ON release_links(rid);
        CREATE INDEX release_producers_rid ON release_producers(rid);
        CREATE INDEX vn_langs_vid_lang ON vn_langs(vid, lang);
        CREATE INDEX vn_tags_vid_tag ON vn_tags(vid, tag);
        ",
    )?;
    Ok(())
}

/// Recompute per-VN fields that come from releases (languages, platforms, first release date,
/// 18+, freeware, developers). `only` limits the work to some VNs (used by incremental sync).
pub fn recompute_derived(c: &Connection, only: Option<&[u32]>) -> Result<()> {
    let (w_vn, w_rv) = match only {
        Some([]) => return Ok(()),
        Some(ids) => {
            let list = ids
                .iter()
                .map(|i| i.to_string())
                .collect::<Vec<_>>()
                .join(",");
            (
                format!("WHERE vn.id IN ({list})"),
                format!("AND rv.vid IN ({list})"),
            )
        }
        None => (String::new(), String::new()),
    };
    let langs_scope = match only {
        Some(_) => w_vn.replace("vn.id", "vid"),
        None => String::new(),
    };
    c.execute_batch(&format!(
        "
        DELETE FROM vn_langs {langs_scope};
        INSERT INTO vn_langs(vid, lang, kind)
          SELECT rv.vid, rt.lang, MIN(CASE WHEN rt.mtl THEN 2 WHEN r.official THEN 0 ELSE 1 END)
          FROM release_vn rv JOIN releases r ON r.id = rv.rid JOIN release_titles rt ON rt.rid = r.id
          WHERE rt.lang <> '' {w_rv}
          GROUP BY rv.vid, rt.lang;
        UPDATE vn_langs SET kind = 0 WHERE EXISTS (SELECT 1 FROM vn WHERE vn.id = vn_langs.vid AND vn.olang = vn_langs.lang);
        INSERT INTO vn_langs(vid, lang, kind)
          SELECT vn.id, vn.olang, 0 FROM vn {w_vn_and} vn.olang <> ''
            AND NOT EXISTS (SELECT 1 FROM vn_langs l WHERE l.vid = vn.id AND l.lang = vn.olang);
        UPDATE vn SET
          released = COALESCE((SELECT MIN(r.released) FROM release_vn rv JOIN releases r ON r.id = rv.rid
                               WHERE rv.vid = vn.id AND rv.rtype <> 'trial' AND r.released BETWEEN 1 AND 99999998), 0),
          has_ero = EXISTS (SELECT 1 FROM release_vn rv JOIN releases r ON r.id = rv.rid WHERE rv.vid = vn.id AND r.has_ero),
          minage = COALESCE((SELECT MAX(r.minage) FROM release_vn rv JOIN releases r ON r.id = rv.rid WHERE rv.vid = vn.id), -1),
          freeware = EXISTS (SELECT 1 FROM release_vn rv JOIN releases r ON r.id = rv.rid
                             WHERE rv.vid = vn.id AND r.freeware AND r.official AND NOT r.patch AND rv.rtype = 'complete'),
          platforms = COALESCE((SELECT group_concat(p, ' ') FROM (
                        SELECT DISTINCT rp.platform AS p FROM release_vn rv JOIN releases r ON r.id = rv.rid
                        JOIN release_platforms rp ON rp.rid = r.id WHERE rv.vid = vn.id AND NOT r.patch ORDER BY p)), ''),
          developers = COALESCE((SELECT group_concat(pid, ' ') FROM (
                        SELECT DISTINCT rpr.pid AS pid FROM release_vn rv JOIN releases r ON r.id = rv.rid
                        JOIN release_producers rpr ON rpr.rid = r.id WHERE rv.vid = vn.id AND r.official AND rpr.developer)), '')
        {w_vn};
        ",
        w_vn_and = if w_vn.is_empty() { "WHERE".to_string() } else { format!("{w_vn} AND") },
    ))?;
    Ok(())
}

pub fn open_catalog(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    Ok(c)
}

pub fn open_catalog_rw(path: &Path) -> Result<Connection> {
    let c = Connection::open(path)?;
    c.busy_timeout(std::time::Duration::from_secs(10))?;
    Ok(c)
}

/// User data (personal shelves) lives in its own file so re-imports never touch it.
pub fn open_user(path: &Path) -> Result<Connection> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let c = Connection::open(path)?;
    c.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS marks(vid INTEGER PRIMARY KEY, status TEXT NOT NULL, note TEXT, updated TEXT DEFAULT (datetime('now')));
        CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY, value TEXT);
        CREATE TABLE IF NOT EXISTS source_index(source TEXT NOT NULL, key TEXT NOT NULL, url TEXT NOT NULL, PRIMARY KEY(source, key));
        CREATE TABLE IF NOT EXISTS translations(src TEXT NOT NULL, lang TEXT NOT NULL, dst TEXT NOT NULL, PRIMARY KEY(src, lang));
        CREATE TABLE IF NOT EXISTS source_meta(source TEXT PRIMARY KEY, fetched_at TEXT, entries INTEGER);
        ",
    )?;
    Ok(c)
}

/// Free bytes on the file system holding `dir` (None where unknown).
#[cfg(unix)]
pub fn free_space(dir: &std::path::Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `c` is a valid NUL-terminated path and `st` a properly sized out-parameter.
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    Some(st.f_bavail as u64 * st.f_frsize as u64)
}

#[cfg(not(unix))]
pub fn free_space(_dir: &std::path::Path) -> Option<u64> {
    None
}
