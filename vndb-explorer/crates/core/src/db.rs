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
        ",
    )?;
    Ok(())
}

pub fn open_catalog(path: &Path) -> Result<Connection> {
    let c = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
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
        ",
    )?;
    Ok(c)
}
