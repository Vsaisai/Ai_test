//! `external.sqlite`: games from external providers, their VNDB matches and provider state.
//!
//! Every game a provider lists is **stored**; what is **shown** is decided separately:
//! * `blocked = 1` — outside the content policy (`hidden_reason` says why). Kept for the import
//!   report (title, developer, tags, version, date) but never returned to the UI; no cover,
//!   description or links are stored for it.
//! * `needs_check = 1` — found only in the forum listing, tags not known yet: shown after its
//!   game page was read and passed the policy.

use super::{policy_reason, ExtGame, OfficialLink};
use anyhow::Result;
use rusqlite::{params, Connection, OptionalExtension, Row};
use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS ext_games(
  source TEXT NOT NULL,
  ext_id TEXT NOT NULL,
  title TEXT NOT NULL DEFAULT '',
  norm_title TEXT NOT NULL DEFAULT '',
  developer TEXT NOT NULL DEFAULT '',
  version TEXT NOT NULL DEFAULT '',
  status TEXT NOT NULL DEFAULT '',
  engine TEXT NOT NULL DEFAULT '',
  tags TEXT NOT NULL DEFAULT '[]',
  cover TEXT,
  rating REAL,
  likes INTEGER NOT NULL DEFAULT 0,
  views INTEGER NOT NULL DEFAULT 0,
  updated TEXT,
  page_url TEXT NOT NULL DEFAULT '',
  description TEXT,
  links TEXT NOT NULL DEFAULT '[]',
  listed_at TEXT,
  details_at TEXT,
  vndb_id INTEGER,
  match_kind TEXT,
  blocked INTEGER NOT NULL DEFAULT 0,
  PRIMARY KEY(source, ext_id)
);
CREATE INDEX IF NOT EXISTS ext_games_vndb ON ext_games(vndb_id);
CREATE TABLE IF NOT EXISTS ext_meta(
  source TEXT NOT NULL,
  key TEXT NOT NULL,
  value TEXT NOT NULL,
  PRIMARY KEY(source, key)
);
";

/// Columns added after the first version (older files get them on open).
const ADDED: &[(&str, &str)] = &[
    ("hidden_reason", "TEXT"),
    ("match_manual", "INTEGER NOT NULL DEFAULT 0"),
    ("found_via", "TEXT NOT NULL DEFAULT 'feed'"),
    ("needs_check", "INTEGER NOT NULL DEFAULT 0"),
];

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    let have: Vec<String> = conn
        .prepare("SELECT name FROM pragma_table_info('ext_games')")?
        .query_map([], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    for (col, ty) in ADDED {
        if !have.iter().any(|c| c == col) {
            conn.execute_batch(&format!("ALTER TABLE ext_games ADD COLUMN {col} {ty}"))?;
        }
    }
    // first version kept nothing but the id of hidden games
    conn.execute_batch(
        "UPDATE ext_games SET hidden_reason = 'policy' WHERE blocked = 1 AND hidden_reason IS NULL",
    )?;
    Ok(())
}

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    migrate(&conn)?;
    Ok(conn)
}

#[cfg(test)]
pub fn open_memory() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    migrate(&conn).unwrap();
    conn
}

/// A stored game with its VNDB match.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    #[serde(flatten)]
    pub game: ExtGame,
    pub vndb_id: Option<u32>,
    /// "link" (same official page as a VNDB release), "title+link", "title+developer",
    /// "title" or "manual"
    pub match_kind: Option<String>,
    /// The match was set (or removed) by the user: automatic matching leaves it alone
    pub match_manual: bool,
    /// When the game page was read (description and links are known)
    pub details_at: Option<String>,
    /// "feed" or "forum"
    pub found_via: String,
}

fn now() -> String {
    // seconds since the epoch are enough to order and age entries
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

pub fn stamp() -> String {
    now()
}

/// Seconds since a stored timestamp (None if never).
pub fn age_secs(stamp: &Option<String>) -> Option<u64> {
    let t: u64 = stamp.as_ref()?.parse().ok()?;
    now().parse::<u64>().ok().map(|n| n.saturating_sub(t))
}

/// What saving a listed game changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    New,
    Updated,
    Unchanged,
    /// Stored, but outside the content policy (reason)
    Hidden(String),
}

struct Before {
    blocked: bool,
    version: String,
    updated: Option<String>,
    title: String,
    tags: String,
}

fn before(conn: &Connection, source: &str, ext_id: &str) -> Result<Option<Before>> {
    Ok(conn
        .query_row(
            "SELECT blocked, version, updated, title, tags FROM ext_games WHERE source = ?1 AND ext_id = ?2",
            params![source, ext_id],
            |r| {
                Ok(Before {
                    blocked: r.get::<_, i64>(0)? == 1,
                    version: r.get(1)?,
                    updated: r.get(2)?,
                    title: r.get(3)?,
                    tags: r.get(4)?,
                })
            },
        )
        .optional()?)
}

fn hide(conn: &Connection, source: &str, ext_id: &str, reason: &str) -> Result<()> {
    // the record stays for the report; nothing that could be displayed is kept
    conn.execute(
        "UPDATE ext_games SET blocked = 1, hidden_reason = ?3, cover = NULL, description = NULL,
            links = '[]', vndb_id = NULL, match_kind = NULL, match_manual = 0
         WHERE source = ?1 AND ext_id = ?2",
        params![source, ext_id, reason],
    )?;
    Ok(())
}

/// Save a game from a catalog listing (`found_via`: "feed" or "forum"). Description and links
/// (from the game page) are kept. A game hidden once stays hidden.
pub fn upsert_listed(conn: &Connection, g: &ExtGame, found_via: &str) -> Result<Change> {
    let prev = before(conn, &g.source, &g.ext_id)?;
    let tags = serde_json::to_string(&g.tags)?;
    // the forum listing has no tags: such a game waits for its page before it is shown
    let unknown_tags = found_via == "forum";
    conn.execute(
        "INSERT INTO ext_games(source, ext_id, title, norm_title, developer, version, status, engine,
            tags, cover, rating, likes, views, updated, page_url, listed_at, found_via, needs_check)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(source, ext_id) DO UPDATE SET
            title = CASE WHEN excluded.title = '' THEN title ELSE excluded.title END,
            norm_title = CASE WHEN excluded.title = '' THEN norm_title ELSE excluded.norm_title END,
            developer = CASE WHEN excluded.developer = '' THEN developer ELSE excluded.developer END,
            version = CASE WHEN excluded.version = '' THEN version ELSE excluded.version END,
            status = CASE WHEN excluded.status = '' THEN status ELSE excluded.status END,
            engine = CASE WHEN excluded.engine = '' THEN engine ELSE excluded.engine END,
            tags = CASE WHEN ?18 THEN tags ELSE excluded.tags END,
            cover = COALESCE(excluded.cover, cover),
            rating = COALESCE(excluded.rating, rating),
            likes = CASE WHEN ?18 THEN likes ELSE excluded.likes END,
            views = CASE WHEN ?18 THEN views ELSE excluded.views END,
            updated = COALESCE(excluded.updated, updated), page_url = excluded.page_url,
            listed_at = excluded.listed_at,
            -- the feed has the tags: a game found in the forum is checked now
            needs_check = CASE WHEN ?18 THEN needs_check ELSE 0 END",
        params![
            g.source,
            g.ext_id,
            g.title,
            super::matching::norm(&g.title),
            g.developer,
            g.version,
            g.status,
            g.engine,
            tags,
            g.cover,
            g.rating,
            g.likes,
            g.views,
            g.updated,
            g.page_url,
            now(),
            found_via,
            unknown_tags
        ],
    )?;
    if let Some(reason) = policy_reason(&g.tags).or_else(|| {
        prev.as_ref()
            .filter(|p| p.blocked)
            .map(|_| "policy".to_string())
    }) {
        let reason = existing_reason(conn, &g.source, &g.ext_id)?.unwrap_or(reason);
        hide(conn, &g.source, &g.ext_id, &reason)?;
        return Ok(Change::Hidden(reason));
    }
    Ok(match prev {
        None => Change::New,
        Some(p)
            if p.version != g.version && !g.version.is_empty()
                || g.updated.is_some() && p.updated != g.updated
                || !g.title.is_empty() && p.title != g.title
                || !unknown_tags && p.tags != tags =>
        {
            Change::Updated
        }
        Some(_) => Change::Unchanged,
    })
}

fn existing_reason(conn: &Connection, source: &str, ext_id: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT hidden_reason FROM ext_games WHERE source = ?1 AND ext_id = ?2 AND blocked = 1",
            params![source, ext_id],
            |r| r.get::<_, Option<String>>(0),
        )
        .optional()?
        .flatten())
}

/// Save everything read from the game page. Fields the page does not have (rating, likes,
/// views) and empty values keep what the catalog listing gave.
pub fn save_details(conn: &Connection, g: &ExtGame) -> Result<Change> {
    let prev = before(conn, &g.source, &g.ext_id)?;
    conn.execute(
        "INSERT OR IGNORE INTO ext_games(source, ext_id, page_url, listed_at, found_via) VALUES(?1, ?2, ?3, ?4, 'page')",
        params![g.source, g.ext_id, g.page_url, now()],
    )?;
    let keep = |v: &str| (!v.is_empty()).then(|| v.to_string());
    conn.execute(
        "UPDATE ext_games SET
            title = COALESCE(?3, title), norm_title = COALESCE(?4, norm_title),
            developer = COALESCE(?5, developer), version = COALESCE(?6, version),
            status = COALESCE(?7, status), engine = COALESCE(?8, engine),
            tags = COALESCE(?9, tags), cover = COALESCE(?10, cover), updated = COALESCE(?11, updated),
            description = ?12, links = ?13, details_at = ?14, needs_check = 0
         WHERE source = ?1 AND ext_id = ?2",
        params![
            g.source,
            g.ext_id,
            keep(&g.title),
            keep(&g.title).map(|t| super::matching::norm(&t)),
            keep(&g.developer),
            keep(&g.version),
            keep(&g.status),
            keep(&g.engine),
            (!g.tags.is_empty())
                .then(|| serde_json::to_string(&g.tags))
                .transpose()?,
            g.cover,
            g.updated,
            g.description,
            serde_json::to_string(&g.links)?,
            now()
        ],
    )?;
    if let Some(reason) = policy_reason(&g.tags).or_else(|| {
        prev.as_ref()
            .filter(|p| p.blocked)
            .map(|_| "policy".to_string())
    }) {
        let reason = existing_reason(conn, &g.source, &g.ext_id)?.unwrap_or(reason);
        hide(conn, &g.source, &g.ext_id, &reason)?;
        return Ok(Change::Hidden(reason));
    }
    Ok(if prev.is_none() {
        Change::New
    } else {
        Change::Updated
    })
}

/// Automatic match; a match set by the user is left alone.
pub fn set_match(
    conn: &Connection,
    source: &str,
    ext_id: &str,
    vndb_id: Option<u32>,
    kind: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE ext_games SET vndb_id = ?3, match_kind = ?4
         WHERE source = ?1 AND ext_id = ?2 AND match_manual = 0 AND blocked = 0",
        params![source, ext_id, vndb_id, kind],
    )?;
    Ok(())
}

/// The user's decision: `Some(vid)` links the game to that VN, `None` says "not in VNDB".
pub fn set_manual_match(
    conn: &Connection,
    source: &str,
    ext_id: &str,
    vndb_id: Option<u32>,
) -> Result<bool> {
    let n = conn.execute(
        "UPDATE ext_games SET vndb_id = ?3, match_kind = 'manual', match_manual = 1
         WHERE source = ?1 AND ext_id = ?2 AND blocked = 0",
        params![source, ext_id, vndb_id],
    )?;
    Ok(n > 0)
}

/// Hand the match back to automatic matching (it is recomputed by the caller).
pub fn clear_manual_match(conn: &Connection, source: &str, ext_id: &str) -> Result<()> {
    conn.execute(
        "UPDATE ext_games SET match_manual = 0, vndb_id = NULL, match_kind = NULL
         WHERE source = ?1 AND ext_id = ?2",
        params![source, ext_id],
    )?;
    Ok(())
}

const COLS: &str = "source, ext_id, title, developer, version, status, engine, tags, cover, rating,
    likes, views, updated, page_url, description, links, vndb_id, match_kind, details_at,
    match_manual, found_via";

/// Shown to the user: inside the policy and checked.
const VISIBLE: &str = "blocked = 0 AND needs_check = 0 AND title != ''";

fn row(r: &Row) -> rusqlite::Result<Stored> {
    let tags: String = r.get(7)?;
    let links: String = r.get(15)?;
    Ok(Stored {
        game: ExtGame {
            source: r.get(0)?,
            ext_id: r.get(1)?,
            title: r.get(2)?,
            developer: r.get(3)?,
            version: r.get(4)?,
            status: r.get(5)?,
            engine: r.get(6)?,
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            cover: r.get(8)?,
            rating: r.get::<_, Option<f64>>(9)?.map(|x| x as f32),
            likes: r.get(10)?,
            views: r.get(11)?,
            updated: r.get(12)?,
            page_url: r.get(13)?,
            description: r.get(14)?,
            links: serde_json::from_str::<Vec<OfficialLink>>(&links).unwrap_or_default(),
        },
        vndb_id: r.get(16)?,
        match_kind: r.get(17)?,
        details_at: r.get(18)?,
        match_manual: r.get::<_, i64>(19)? == 1,
        found_via: r.get(20)?,
    })
}

/// A game the user may see (None for hidden or unchecked ones).
pub fn get(conn: &Connection, source: &str, ext_id: &str) -> Result<Option<Stored>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM ext_games WHERE source = ?1 AND ext_id = ?2 AND {VISIBLE}"
            ),
            params![source, ext_id],
            row,
        )
        .optional()?)
}

pub fn is_blocked(conn: &Connection, source: &str, ext_id: &str) -> Result<bool> {
    Ok(conn
        .query_row(
            "SELECT blocked FROM ext_games WHERE source = ?1 AND ext_id = ?2",
            params![source, ext_id],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
        == Some(1))
}

/// Games found in the forum whose page was not read yet (oldest first).
pub fn unchecked(conn: &Connection, source: &str, limit: usize) -> Result<Vec<String>> {
    let mut st = conn.prepare(
        "SELECT ext_id FROM ext_games WHERE source = ?1 AND needs_check = 1 AND blocked = 0
         ORDER BY listed_at LIMIT ?2",
    )?;
    let out = st
        .query_map(params![source, limit as i64], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(out)
}

/// Every stored id of a source (hidden ones included), for the forum comparison.
pub fn all_ids(conn: &Connection, source: &str) -> Result<std::collections::HashSet<String>> {
    let mut st = conn.prepare("SELECT ext_id FROM ext_games WHERE source = ?1")?;
    let out = st
        .query_map([source], |r| r.get(0))?
        .filter_map(Result::ok)
        .collect();
    Ok(out)
}

/// Games matched to a VNDB entry (for the VN page).
pub fn for_vndb(conn: &Connection, vid: u32) -> Result<Vec<Stored>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM ext_games WHERE vndb_id = ?1 AND {VISIBLE}"
    ))?;
    let out = st.query_map([vid], row)?.filter_map(Result::ok).collect();
    Ok(out)
}

/// Every visible game.
pub fn all(conn: &Connection) -> Result<Vec<Stored>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM ext_games WHERE {VISIBLE}"))?;
    let out = st.query_map([], row)?.filter_map(Result::ok).collect();
    Ok(out)
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(default)]
pub struct Query {
    pub source: Option<String>,
    /// Words of the title or developer
    pub q: String,
    /// Every tag must be present
    pub tags: Vec<String>,
    pub exclude_tags: Vec<String>,
    pub status: Option<String>,
    pub engine: Option<String>,
    /// Only games that are not in VNDB
    pub only_new: bool,
    /// VNDB match: "weak" (by title only — worth checking), "manual", "none", "any"
    pub matches: Option<String>,
    /// "updated" (default), "rating", "likes", "title"
    pub sort: Option<String>,
    pub page: u32,
    pub per_page: u32,
}

#[derive(Debug, Serialize)]
pub struct Page {
    pub total: usize,
    pub page: u32,
    pub per_page: u32,
    pub items: Vec<Stored>,
}

pub fn search(conn: &Connection, q: &Query) -> Result<Page> {
    let per_page = if q.per_page == 0 {
        24
    } else {
        q.per_page.min(120)
    };
    let page = q.page.max(1);
    let words: Vec<String> = super::matching::norm(&q.q)
        .split_whitespace()
        .map(String::from)
        .collect();
    let mut all: Vec<Stored> = all(conn)?
        .into_iter()
        .filter(|s| {
            let g = &s.game;
            if let Some(src) = &q.source {
                if &g.source != src {
                    return false;
                }
            }
            if !words.is_empty() {
                let hay = format!(
                    "{} {}",
                    super::matching::norm(&g.title),
                    super::matching::norm(&g.developer)
                );
                if !words.iter().all(|w| hay.contains(w.as_str())) {
                    return false;
                }
            }
            let has = |t: &String| g.tags.iter().any(|x| x.eq_ignore_ascii_case(t));
            if !q.tags.iter().all(has) || q.exclude_tags.iter().any(has) {
                return false;
            }
            if let Some(st) = q.status.as_deref().filter(|s| !s.is_empty()) {
                if g.status != st {
                    return false;
                }
            }
            if let Some(en) = q.engine.as_deref().filter(|s| !s.is_empty()) {
                if !g.engine.eq_ignore_ascii_case(en) {
                    return false;
                }
            }
            match q.matches.as_deref() {
                Some("weak") if s.match_manual || s.match_kind.as_deref() != Some("title") => {
                    return false
                }
                Some("manual") if !s.match_manual => return false,
                Some("none") if s.vndb_id.is_some() => return false,
                Some("any") if s.vndb_id.is_none() => return false,
                _ => {}
            }
            !(q.only_new && s.vndb_id.is_some())
        })
        .collect();
    match q.sort.as_deref() {
        Some("rating") => all.sort_by(|a, b| {
            b.game
                .rating
                .unwrap_or(0.0)
                .total_cmp(&a.game.rating.unwrap_or(0.0))
                .then(b.game.likes.cmp(&a.game.likes))
        }),
        Some("likes") => all.sort_by_key(|b| std::cmp::Reverse(b.game.likes)),
        Some("title") => all.sort_by_key(|a| a.game.title.to_lowercase()),
        _ => all.sort_by(|a, b| b.game.updated.cmp(&a.game.updated)),
    }
    let total = all.len();
    let items = all
        .into_iter()
        .skip(((page - 1) * per_page) as usize)
        .take(per_page as usize)
        .collect();
    Ok(Page {
        total,
        page,
        per_page,
        items,
    })
}

/// Tags with the number of games (most common first), for the filter.
pub fn tag_counts(conn: &Connection, source: &str) -> Result<Vec<(String, usize)>> {
    let mut st = conn.prepare(&format!(
        "SELECT tags FROM ext_games WHERE source = ?1 AND {VISIBLE}"
    ))?;
    let mut counts: HashMap<String, usize> = HashMap::new();
    for tags in st.query_map([source], |r| r.get::<_, String>(0))? {
        for t in serde_json::from_str::<Vec<String>>(&tags?).unwrap_or_default() {
            *counts.entry(t).or_default() += 1;
        }
    }
    let mut v: Vec<_> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(v)
}

/// Extra records sharing a title and developer with another one (possible double threads).
pub fn duplicate_count(conn: &Connection, source: &str) -> Result<usize> {
    Ok(conn.query_row(
        &format!(
            "SELECT COALESCE(SUM(n - 1), 0) FROM (
               SELECT COUNT(*) AS n FROM ext_games WHERE source = ?1 AND {VISIBLE} AND norm_title != ''
               GROUP BY norm_title, lower(developer) HAVING n > 1)"
        ),
        [source],
        |r| r.get::<_, i64>(0),
    )? as usize)
}

pub fn set_meta(conn: &Connection, source: &str, key: &str, value: &str) -> Result<()> {
    conn.execute(
        "INSERT INTO ext_meta(source, key, value) VALUES(?1, ?2, ?3)
         ON CONFLICT(source, key) DO UPDATE SET value = excluded.value",
        params![source, key, value],
    )?;
    Ok(())
}

pub fn get_meta(conn: &Connection, source: &str, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT value FROM ext_meta WHERE source = ?1 AND key = ?2",
            params![source, key],
            |r| r.get(0),
        )
        .optional()?)
}

pub fn del_meta(conn: &Connection, source: &str, key: &str) -> Result<()> {
    conn.execute(
        "DELETE FROM ext_meta WHERE source = ?1 AND key = ?2",
        params![source, key],
    )?;
    Ok(())
}

#[derive(Debug, Serialize)]
pub struct Stats {
    /// Every stored record (shown + hidden + waiting for a check)
    pub stored: usize,
    /// Shown to the user
    pub games: usize,
    pub in_vndb: usize,
    pub manual_matches: usize,
    pub weak_matches: usize,
    pub with_details: usize,
    pub needs_check: usize,
    pub hidden_by_policy: usize,
    /// Hidden records by reason (tag)
    pub hidden_reasons: HashMap<String, usize>,
    pub duplicates: usize,
    pub listed_at: Option<String>,
    pub last_error: Option<String>,
    /// When a complete pass over the whole feed last finished
    pub feed_complete_at: Option<String>,
    pub last_report: Option<serde_json::Value>,
    pub last_audit: Option<serde_json::Value>,
}

pub fn stats(conn: &Connection, source: &str) -> Result<Stats> {
    let n = |sql: &str| -> Result<usize> {
        Ok(conn.query_row(
            &format!("SELECT COUNT(*) FROM ext_games WHERE source = ?1 AND {sql}"),
            [source],
            |r| r.get::<_, i64>(0),
        )? as usize)
    };
    let mut reasons = HashMap::new();
    {
        let mut st = conn.prepare(
            "SELECT COALESCE(hidden_reason, 'policy'), COUNT(*) FROM ext_games
             WHERE source = ?1 AND blocked = 1 GROUP BY 1",
        )?;
        for r in st.query_map([source], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })? {
            let (k, v) = r?;
            reasons.insert(k, v as usize);
        }
    }
    let json_meta = |k: &str| -> Result<Option<serde_json::Value>> {
        Ok(get_meta(conn, source, k)?.and_then(|s| serde_json::from_str(&s).ok()))
    };
    Ok(Stats {
        stored: n("1")?,
        games: n(VISIBLE)?,
        in_vndb: n(&format!("{VISIBLE} AND vndb_id IS NOT NULL"))?,
        manual_matches: n(&format!("{VISIBLE} AND match_manual = 1"))?,
        weak_matches: n(&format!(
            "{VISIBLE} AND match_manual = 0 AND match_kind = 'title'"
        ))?,
        with_details: n(&format!("{VISIBLE} AND details_at IS NOT NULL"))?,
        needs_check: n("blocked = 0 AND needs_check = 1")?,
        hidden_by_policy: n("blocked = 1")?,
        hidden_reasons: reasons,
        duplicates: duplicate_count(conn, source)?,
        listed_at: get_meta(conn, source, "listed_at")?,
        last_error: get_meta(conn, source, "last_error")?.filter(|e| !e.is_empty()),
        feed_complete_at: get_meta(conn, source, "feed_complete_at")?,
        last_report: json_meta("last_report")?,
        last_audit: json_meta("last_audit")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(id: &str, title: &str, tags: &[&str]) -> ExtGame {
        ExtGame {
            source: "f95zone".into(),
            ext_id: id.into(),
            title: title.into(),
            developer: "Dev".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            page_url: format!("https://f95zone.to/threads/{id}/"),
            updated: Some("2026-11-01".into()),
            cover: Some("https://attachments.f95zone.to/x.png".into()),
            ..Default::default()
        }
    }

    #[test]
    fn hidden_games_are_stored_and_counted_but_never_shown() {
        let conn = open_memory();
        assert_eq!(
            upsert_listed(&conn, &game("1", "Fine Game", &["3dcg"]), "feed").unwrap(),
            Change::New
        );
        assert_eq!(
            upsert_listed(&conn, &game("2", "Bad Game", &["3dcg", "loli"]), "feed").unwrap(),
            Change::Hidden("loli".into())
        );
        assert!(get(&conn, "f95zone", "2").unwrap().is_none());
        // the record is kept for the report, without anything displayable
        let (title, cover, reason): (String, Option<String>, String) = conn
            .query_row(
                "SELECT title, cover, hidden_reason FROM ext_games WHERE ext_id = '2'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            (title.as_str(), cover, reason.as_str()),
            ("Bad Game", None, "loli")
        );
        // a later listing without the tag does not bring it back
        assert!(matches!(
            upsert_listed(&conn, &game("2", "Bad Game", &["3dcg"]), "feed").unwrap(),
            Change::Hidden(_)
        ));
        assert!(get(&conn, "f95zone", "2").unwrap().is_none());
        // a game page revealing the tag hides a listed game
        assert_eq!(
            save_details(&conn, &game("1", "Fine Game", &["shota"])).unwrap(),
            Change::Hidden("shota".into())
        );
        assert!(get(&conn, "f95zone", "1").unwrap().is_none());
        assert_eq!(search(&conn, &Query::default()).unwrap().total, 0);
        let s = stats(&conn, "f95zone").unwrap();
        assert_eq!((s.stored, s.games, s.hidden_by_policy), (2, 0, 2));
        assert_eq!(s.hidden_reasons.get("loli"), Some(&1));
        assert_eq!(s.hidden_reasons.get("shota"), Some(&1));
    }

    #[test]
    fn forum_finds_wait_for_their_page() {
        let conn = open_memory();
        let mut g = game("5", "Forum Only", &[]);
        g.cover = None;
        assert_eq!(upsert_listed(&conn, &g, "forum").unwrap(), Change::New);
        assert!(get(&conn, "f95zone", "5").unwrap().is_none());
        assert_eq!(unchecked(&conn, "f95zone", 10).unwrap(), vec!["5"]);
        save_details(&conn, &game("5", "Forum Only", &["2dcg"])).unwrap();
        assert!(get(&conn, "f95zone", "5").unwrap().is_some());
        assert!(unchecked(&conn, "f95zone", 10).unwrap().is_empty());
        // the forum listing does not wipe tags known from the feed
        upsert_listed(&conn, &game("6", "Both", &["3dcg"]), "feed").unwrap();
        let mut f = game("6", "Both", &[]);
        f.cover = None;
        assert_eq!(
            upsert_listed(&conn, &f, "forum").unwrap(),
            Change::Unchanged
        );
        assert_eq!(
            get(&conn, "f95zone", "6").unwrap().unwrap().game.tags,
            vec!["3dcg"]
        );
    }

    #[test]
    fn changes_are_detected() {
        let conn = open_memory();
        upsert_listed(&conn, &game("7", "A", &["x"]), "feed").unwrap();
        assert_eq!(
            upsert_listed(&conn, &game("7", "A", &["x"]), "feed").unwrap(),
            Change::Unchanged
        );
        let mut g = game("7", "A", &["x"]);
        g.version = "v2".into();
        assert_eq!(upsert_listed(&conn, &g, "feed").unwrap(), Change::Updated);
        assert_eq!(upsert_listed(&conn, &g, "feed").unwrap(), Change::Unchanged);
        g.tags.push("y".into());
        assert_eq!(upsert_listed(&conn, &g, "feed").unwrap(), Change::Updated);
    }

    #[test]
    fn manual_matches_survive_automatic_matching() {
        let conn = open_memory();
        upsert_listed(&conn, &game("8", "Eternum", &[]), "feed").unwrap();
        set_match(&conn, "f95zone", "8", Some(1), Some("title")).unwrap();
        assert_eq!(
            search(
                &conn,
                &Query {
                    matches: Some("weak".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .total,
            1
        );
        // the user says: not this VN
        assert!(set_manual_match(&conn, "f95zone", "8", None).unwrap());
        set_match(&conn, "f95zone", "8", Some(1), Some("title")).unwrap();
        let s = get(&conn, "f95zone", "8").unwrap().unwrap();
        assert_eq!((s.vndb_id, s.match_manual), (None, true));
        assert_eq!(
            search(
                &conn,
                &Query {
                    matches: Some("manual".into()),
                    ..Default::default()
                }
            )
            .unwrap()
            .total,
            1
        );
        // the user picks the right one
        set_manual_match(&conn, "f95zone", "8", Some(31929)).unwrap();
        assert_eq!(for_vndb(&conn, 31929).unwrap().len(), 1);
        // back to automatic
        clear_manual_match(&conn, "f95zone", "8").unwrap();
        set_match(&conn, "f95zone", "8", Some(1), Some("title")).unwrap();
        assert_eq!(
            get(&conn, "f95zone", "8").unwrap().unwrap().vndb_id,
            Some(1)
        );
    }

    #[test]
    fn details_survive_relisting_and_search_filters() {
        let conn = open_memory();
        let mut listed = game("7", "Summer Days", &["3dcg", "romance"]);
        listed.rating = Some(4.5);
        listed.likes = 12;
        upsert_listed(&conn, &listed, "feed").unwrap();
        let mut g = game("7", "Summer Days", &["3dcg", "romance"]);
        g.description = Some("About summer".into());
        g.links = vec![OfficialLink {
            site: "patreon".into(),
            label: "Patreon".into(),
            url: "https://www.patreon.com/dev".into(),
        }];
        save_details(&conn, &g).unwrap();
        let after = get(&conn, "f95zone", "7").unwrap().unwrap();
        assert_eq!((after.game.likes, after.game.rating), (12, Some(4.5)));
        upsert_listed(
            &conn,
            &game("7", "Summer Days", &["3dcg", "romance"]),
            "feed",
        )
        .unwrap();
        let s = get(&conn, "f95zone", "7").unwrap().unwrap();
        assert_eq!(s.game.description.as_deref(), Some("About summer"));
        assert_eq!(s.game.links.len(), 1);
        upsert_listed(&conn, &game("8", "Winter Nights", &["2dcg"]), "feed").unwrap();
        let q = |q: Query| search(&conn, &q).unwrap().total;
        assert_eq!(
            q(Query {
                q: "summer".into(),
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Query {
                tags: vec!["3DCG".into()],
                ..Default::default()
            }),
            1
        );
        assert_eq!(
            q(Query {
                exclude_tags: vec!["3dcg".into()],
                ..Default::default()
            }),
            1
        );
        assert_eq!(q(Query::default()), 2);
        set_match(&conn, "f95zone", "7", Some(42), Some("title")).unwrap();
        assert_eq!(
            q(Query {
                only_new: true,
                ..Default::default()
            }),
            1
        );
        assert_eq!(for_vndb(&conn, 42).unwrap().len(), 1);
        // same title and developer twice: one duplicate
        upsert_listed(&conn, &game("9", "Winter Nights", &["2dcg"]), "feed").unwrap();
        assert_eq!(duplicate_count(&conn, "f95zone").unwrap(), 1);
    }

    #[test]
    fn old_files_are_migrated() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        conn.execute(
            "INSERT INTO ext_games(source, ext_id, blocked) VALUES('f95zone', '1', 1)",
            [],
        )
        .unwrap();
        migrate(&conn).unwrap();
        let s = stats(&conn, "f95zone").unwrap();
        assert_eq!(s.hidden_reasons.get("policy"), Some(&1));
    }
}
