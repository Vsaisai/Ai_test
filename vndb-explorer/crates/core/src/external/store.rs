//! `external.sqlite`: games from external providers, their VNDB matches and provider state.

use super::{is_forbidden, ExtGame, OfficialLink};
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

pub fn open(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch(SCHEMA)?;
    Ok(conn)
}

/// A stored game with its VNDB match.
#[derive(Debug, Clone, Serialize)]
pub struct Stored {
    #[serde(flatten)]
    pub game: ExtGame,
    pub vndb_id: Option<u32>,
    /// "link" (same official page as a VNDB release), "title+developer" or "title"
    pub match_kind: Option<String>,
    /// When the game page was read (description and links are known)
    pub details_at: Option<String>,
}

fn now() -> String {
    // seconds since the epoch are enough to order and age entries
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

/// Seconds since a stored timestamp (None if never).
pub fn age_secs(stamp: &Option<String>) -> Option<u64> {
    let t: u64 = stamp.as_ref()?.parse().ok()?;
    now().parse::<u64>().ok().map(|n| n.saturating_sub(t))
}

fn block(conn: &Connection, source: &str, ext_id: &str) -> Result<()> {
    // keep only the id, so the game is skipped next time; no title, cover or description
    conn.execute(
        "INSERT INTO ext_games(source, ext_id, blocked, listed_at) VALUES(?1, ?2, 1, ?3)
         ON CONFLICT(source, ext_id) DO UPDATE SET blocked = 1, title = '', norm_title = '',
           developer = '', tags = '[]', cover = NULL, description = NULL, links = '[]',
           vndb_id = NULL, match_kind = NULL",
        params![source, ext_id, now()],
    )?;
    Ok(())
}

/// Save a game from a catalog page. Description and links (from the game page) are kept.
/// Returns true for a game not seen before.
pub fn upsert_listed(conn: &Connection, g: &ExtGame) -> Result<bool> {
    let existed: Option<i64> = conn
        .query_row(
            "SELECT blocked FROM ext_games WHERE source = ?1 AND ext_id = ?2",
            params![g.source, g.ext_id],
            |r| r.get(0),
        )
        .optional()?;
    if is_forbidden(&g.tags) || existed == Some(1) {
        block(conn, &g.source, &g.ext_id)?;
        return Ok(false);
    }
    conn.execute(
        "INSERT INTO ext_games(source, ext_id, title, norm_title, developer, version, status, engine,
            tags, cover, rating, likes, views, updated, page_url, listed_at)
         VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
         ON CONFLICT(source, ext_id) DO UPDATE SET title = excluded.title,
            norm_title = excluded.norm_title, developer = excluded.developer,
            version = excluded.version,
            status = CASE WHEN excluded.status = '' THEN status ELSE excluded.status END,
            engine = CASE WHEN excluded.engine = '' THEN engine ELSE excluded.engine END,
            tags = excluded.tags, cover = COALESCE(excluded.cover, cover),
            rating = excluded.rating, likes = excluded.likes, views = excluded.views,
            updated = COALESCE(excluded.updated, updated), page_url = excluded.page_url,
            listed_at = excluded.listed_at",
        params![
            g.source,
            g.ext_id,
            g.title,
            super::matching::norm(&g.title),
            g.developer,
            g.version,
            g.status,
            g.engine,
            serde_json::to_string(&g.tags)?,
            g.cover,
            g.rating,
            g.likes,
            g.views,
            g.updated,
            g.page_url,
            now()
        ],
    )?;
    Ok(existed.is_none())
}

/// Save everything read from the game page. Fields the page does not have (rating, likes,
/// views) and empty values keep what the catalog listing gave.
pub fn save_details(conn: &Connection, g: &ExtGame) -> Result<()> {
    if is_forbidden(&g.tags) {
        return block(conn, &g.source, &g.ext_id);
    }
    if is_blocked(conn, &g.source, &g.ext_id)? {
        return Ok(());
    }
    conn.execute(
        "INSERT OR IGNORE INTO ext_games(source, ext_id, page_url, listed_at) VALUES(?1, ?2, ?3, ?4)",
        params![g.source, g.ext_id, g.page_url, now()],
    )?;
    let keep = |v: &str| (!v.is_empty()).then(|| v.to_string());
    conn.execute(
        "UPDATE ext_games SET
            title = COALESCE(?3, title), norm_title = COALESCE(?4, norm_title),
            developer = COALESCE(?5, developer), version = COALESCE(?6, version),
            status = COALESCE(?7, status), engine = COALESCE(?8, engine),
            tags = COALESCE(?9, tags), cover = COALESCE(?10, cover), updated = COALESCE(?11, updated),
            description = ?12, links = ?13, details_at = ?14
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
    Ok(())
}

pub fn set_match(
    conn: &Connection,
    source: &str,
    ext_id: &str,
    vndb_id: Option<u32>,
    kind: Option<&str>,
) -> Result<()> {
    conn.execute(
        "UPDATE ext_games SET vndb_id = ?3, match_kind = ?4 WHERE source = ?1 AND ext_id = ?2",
        params![source, ext_id, vndb_id, kind],
    )?;
    Ok(())
}

const COLS: &str = "source, ext_id, title, developer, version, status, engine, tags, cover, rating,
    likes, views, updated, page_url, description, links, vndb_id, match_kind, details_at";

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
    })
}

pub fn get(conn: &Connection, source: &str, ext_id: &str) -> Result<Option<Stored>> {
    Ok(conn
        .query_row(
            &format!(
                "SELECT {COLS} FROM ext_games WHERE source = ?1 AND ext_id = ?2 AND blocked = 0"
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

/// Games matched to a VNDB entry (for the VN page).
pub fn for_vndb(conn: &Connection, vid: u32) -> Result<Vec<Stored>> {
    let mut st = conn.prepare(&format!(
        "SELECT {COLS} FROM ext_games WHERE vndb_id = ?1 AND blocked = 0"
    ))?;
    let out = st.query_map([vid], row)?.filter_map(Result::ok).collect();
    Ok(out)
}

/// Every game (for matching), without blocked ones.
pub fn all(conn: &Connection) -> Result<Vec<Stored>> {
    let mut st = conn.prepare(&format!("SELECT {COLS} FROM ext_games WHERE blocked = 0"))?;
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
            if g.title.is_empty() {
                return false;
            }
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
    let mut st = conn.prepare("SELECT tags FROM ext_games WHERE source = ?1 AND blocked = 0")?;
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

#[derive(Debug, Serialize)]
pub struct Stats {
    pub games: usize,
    pub in_vndb: usize,
    pub with_details: usize,
    pub hidden_by_policy: usize,
    pub listed_at: Option<String>,
    pub last_error: Option<String>,
}

pub fn stats(conn: &Connection, source: &str) -> Result<Stats> {
    let (games, in_vndb, with_details, hidden): (i64, i64, i64, i64) = conn.query_row(
        "SELECT COALESCE(SUM(blocked = 0), 0), COALESCE(SUM(blocked = 0 AND vndb_id IS NOT NULL), 0),
                COALESCE(SUM(blocked = 0 AND details_at IS NOT NULL), 0), COALESCE(SUM(blocked = 1), 0)
         FROM ext_games WHERE source = ?1",
        [source],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
    )?;
    Ok(Stats {
        games: games as usize,
        in_vndb: in_vndb as usize,
        with_details: with_details as usize,
        hidden_by_policy: hidden as usize,
        listed_at: get_meta(conn, source, "listed_at")?,
        last_error: get_meta(conn, source, "last_error")?,
    })
}

pub fn stamp() -> String {
    now()
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
            ..Default::default()
        }
    }

    #[test]
    fn forbidden_games_keep_only_their_id() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        assert!(upsert_listed(&conn, &game("1", "Fine Game", &["3dcg"])).unwrap());
        upsert_listed(&conn, &game("2", "Bad Game", &["3dcg", "loli"])).unwrap();
        assert!(get(&conn, "f95zone", "2").unwrap().is_none());
        assert!(is_blocked(&conn, "f95zone", "2").unwrap());
        let title: String = conn
            .query_row("SELECT title FROM ext_games WHERE ext_id = '2'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(title, "");
        // a later listing without the tag does not bring it back
        upsert_listed(&conn, &game("2", "Bad Game", &["3dcg"])).unwrap();
        assert!(get(&conn, "f95zone", "2").unwrap().is_none());
        // a game page revealing the tag blocks a listed game
        save_details(&conn, &game("1", "Fine Game", &["shota"])).unwrap();
        assert!(get(&conn, "f95zone", "1").unwrap().is_none());
        let s = stats(&conn, "f95zone").unwrap();
        assert_eq!((s.games, s.hidden_by_policy), (0, 2));
    }

    #[test]
    fn details_survive_relisting_and_search_filters() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(SCHEMA).unwrap();
        let mut listed = game("7", "Summer Days", &["3dcg", "romance"]);
        listed.rating = Some(4.5);
        listed.likes = 12;
        upsert_listed(&conn, &listed).unwrap();
        let mut g = game("7", "Summer Days", &["3dcg", "romance"]);
        g.description = Some("About summer".into());
        g.links = vec![OfficialLink {
            site: "patreon".into(),
            label: "Patreon".into(),
            url: "https://www.patreon.com/dev".into(),
        }];
        save_details(&conn, &g).unwrap();
        // the game page has no likes or rating: the listing's values stay
        let after = get(&conn, "f95zone", "7").unwrap().unwrap();
        assert_eq!((after.game.likes, after.game.rating), (12, Some(4.5)));
        upsert_listed(&conn, &game("7", "Summer Days", &["3dcg", "romance"])).unwrap();
        let s = get(&conn, "f95zone", "7").unwrap().unwrap();
        assert_eq!(s.game.description.as_deref(), Some("About summer"));
        assert_eq!(s.game.links.len(), 1);
        upsert_listed(&conn, &game("8", "Winter Nights", &["2dcg"])).unwrap();
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
    }
}
