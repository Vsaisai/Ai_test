//! Game catalogs outside VNDB ("providers"), kept in `external.sqlite` next to the VNDB catalog.
//!
//! A provider only collects **metadata** (title, developer, description, tags, version, status,
//! cover address, page address and the developer's official pages). Game files, download links,
//! mirrors and repacks are never stored or shown. A game being listed by a provider says nothing
//! about whether its distribution is legal; the app always sends the user to the original page.
//!
//! * [`Provider`] — the replaceable interface (one implementation: [`f95zone::F95Zone`])
//! * [`store`] — SQLite storage, search, the content policy
//! * [`matching`] — links games to VNDB entries and merges duplicates

pub mod f95zone;
pub mod matching;
pub mod store;

use serde::{Deserialize, Serialize};

/// One game as a provider describes it. Empty strings / `None` mean "not known (yet)".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExtGame {
    pub source: String,
    pub ext_id: String,
    pub title: String,
    pub developer: String,
    pub version: String,
    /// "ongoing", "completed", "onhold", "abandoned" or "" (unknown)
    pub status: String,
    /// Engine / format, e.g. "Ren'Py", "Unity", "RPGM"
    pub engine: String,
    pub tags: Vec<String>,
    pub cover: Option<String>,
    /// Community rating 0-5 (as the provider reports it)
    pub rating: Option<f32>,
    pub likes: u32,
    pub views: u32,
    /// Last update of the game, YYYY-MM-DD
    pub updated: Option<String>,
    /// The original publication page
    pub page_url: String,
    /// Short description (filled from the game page)
    pub description: Option<String>,
    /// Official pages of the developer / stores (filled from the game page)
    pub links: Vec<OfficialLink>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OfficialLink {
    /// "patreon", "itch", "steam", "subscribestar", "dlsite", ...
    pub site: String,
    pub label: String,
    pub url: String,
}

/// Why a provider request did not work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderError {
    /// The site refused automated access (rate limit, anti-bot check, login wall). The import
    /// stops; it is never bypassed.
    Blocked(String),
    /// Network error or the site is down; worth trying again later.
    Unavailable(String),
    /// The answer did not have the expected shape (the site changed).
    Parse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Blocked(m) => write!(f, "сайт ограничил доступ: {m}"),
            ProviderError::Unavailable(m) => write!(f, "сайт недоступен: {m}"),
            ProviderError::Parse(m) => write!(f, "не удалось разобрать ответ сайта: {m}"),
        }
    }
}

impl std::error::Error for ProviderError {}

/// A page of the provider's catalog, newest updates first.
#[derive(Debug, Clone, Default)]
pub struct ListPage {
    pub games: Vec<ExtGame>,
    /// Total number of pages, when the provider says so
    pub total_pages: Option<u32>,
}

/// A source of game metadata. Implementations must respect the site's rules: no captcha or
/// anti-bot bypass, polite request pacing, metadata only.
pub trait Provider {
    /// Stable id stored with every game ("f95zone")
    fn id(&self) -> &'static str;
    /// Name shown to the user
    fn name(&self) -> &'static str;
    /// Prepare the provider (e.g. load tag dictionaries). Called once before listing.
    fn prepare(&mut self) -> Result<(), ProviderError> {
        Ok(())
    }
    /// One page (1-based) of the catalog, newest updates first.
    fn list(&mut self, page: u32) -> Result<ListPage, ProviderError>;
    /// Full metadata of one game from its page (description, official links...).
    fn details(&mut self, ext_id: &str) -> Result<ExtGame, ProviderError>;
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RefreshReport {
    pub pages: u32,
    pub seen: usize,
    pub new: usize,
    pub updated: usize,
    pub hidden_by_policy: usize,
    pub in_vndb: Option<usize>,
    /// Set when the provider stopped the import (rate limit, captcha, login wall...)
    pub stopped: Option<String>,
}

/// Read the provider's catalog, newest updates first.
///
/// `full`: read every page (up to `max_pages`); otherwise stop after the first page on which
/// nothing is new or changed. A refusal by the site ends the import with what was read so far.
pub fn refresh(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
    full: bool,
    max_pages: u32,
    log: &dyn Fn(&str),
) -> anyhow::Result<RefreshReport> {
    let mut rep = RefreshReport::default();
    let src = provider.id();
    let fail = |rep: &mut RefreshReport, e: ProviderError| -> anyhow::Result<()> {
        let msg = e.to_string();
        store::set_meta(conn, src, "last_error", &msg)?;
        log(&format!("{}: {msg}", provider_name(src)));
        rep.stopped = Some(msg);
        Ok(())
    };
    if let Err(e) = provider.prepare() {
        fail(&mut rep, e)?;
        return Ok(rep);
    }
    let mut page = 1;
    while page <= max_pages {
        let list = match provider.list(page) {
            Ok(l) => l,
            Err(e) => {
                fail(&mut rep, e)?;
                break;
            }
        };
        if list.games.is_empty() {
            break;
        }
        rep.pages = page;
        let mut changed = 0;
        let tx = conn.unchecked_transaction()?;
        for g in &list.games {
            rep.seen += 1;
            let before = store::get(&tx, &g.source, &g.ext_id)?;
            if is_forbidden(&g.tags) || store::is_blocked(&tx, &g.source, &g.ext_id)? {
                store::upsert_listed(&tx, g)?;
                rep.hidden_by_policy += 1;
                continue;
            }
            store::upsert_listed(&tx, g)?;
            match before {
                None => {
                    rep.new += 1;
                    changed += 1;
                }
                Some(b) if b.game.version != g.version || b.game.updated != g.updated => {
                    rep.updated += 1;
                    changed += 1;
                }
                _ => {}
            }
        }
        tx.commit()?;
        log(&format!(
            "{}: страница {page}{} — новых {}, обновлённых {}",
            provider.name(),
            list.total_pages
                .map(|t| format!(" из {t}"))
                .unwrap_or_default(),
            rep.new,
            rep.updated
        ));
        if !full && changed == 0 {
            break;
        }
        if list.total_pages.is_some_and(|t| page >= t) {
            break;
        }
        page += 1;
    }
    if rep.stopped.is_none() {
        store::set_meta(conn, src, "last_error", "")?;
    }
    store::set_meta(conn, src, "listed_at", &store::stamp())?;
    Ok(rep)
}

fn provider_name(id: &str) -> &str {
    match id {
        f95zone::ID => "F95Zone",
        other => other,
    }
}

/// Tags that put a game outside this app's content policy (sexual content involving minors).
/// Such games are never shown: only their id is kept so they are not fetched again.
pub const FORBIDDEN_TAGS: &[&str] = &["loli", "shota"];

pub fn is_forbidden(tags: &[String]) -> bool {
    tags.iter().any(|t| {
        let t = t.trim().to_lowercase();
        FORBIDDEN_TAGS.iter().any(|f| t == *f)
    })
}

/// Official pages of developers and stores; anything else (file hosts, mirrors, repacks,
/// forum redirects) is dropped.
pub fn official_site(url: &str) -> Option<&'static str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let host = rest
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .to_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    let is = |d: &str| host == d || host.ends_with(&format!(".{d}"));
    Some(if is("patreon.com") {
        "patreon"
    } else if is("subscribestar.adult") || is("subscribestar.com") {
        "subscribestar"
    } else if is("itch.io") {
        "itch"
    } else if host == "store.steampowered.com" {
        "steam"
    } else if is("dlsite.com") {
        "dlsite"
    } else if is("gumroad.com") {
        "gumroad"
    } else if is("ko-fi.com") {
        "kofi"
    } else if is("fanbox.cc") {
        "fanbox"
    } else if is("fantia.jp") {
        "fantia"
    } else if is("gog.com") {
        "gog"
    } else if is("nutaku.net") {
        "nutaku"
    } else if is("johren.games") {
        "johren"
    } else if is("buymeacoffee.com") {
        "buymeacoffee"
    } else if is("x.com") || is("twitter.com") {
        "x"
    } else if is("discord.gg") || is("discord.com") {
        "discord"
    } else {
        return None;
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_sites_only() {
        assert_eq!(
            official_site("https://www.patreon.com/c/dev"),
            Some("patreon")
        );
        assert_eq!(official_site("https://dev.itch.io/game"), Some("itch"));
        assert_eq!(
            official_site("https://store.steampowered.com/app/1"),
            Some("steam")
        );
        assert_eq!(official_site("https://mega.nz/file/x"), None);
        assert_eq!(official_site("https://f95zone.to/masked/abc"), None);
        assert_eq!(official_site("https://evilpatreon.com.example/"), None);
        assert_eq!(official_site("ftp://patreon.com"), None);
    }

    struct Fake {
        pages: Vec<Vec<ExtGame>>,
        fail_at: Option<u32>,
        calls: u32,
    }

    impl Provider for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn list(&mut self, page: u32) -> Result<ListPage, ProviderError> {
            self.calls += 1;
            if self.fail_at == Some(page) {
                return Err(ProviderError::Blocked("captcha".into()));
            }
            Ok(ListPage {
                games: self
                    .pages
                    .get(page as usize - 1)
                    .cloned()
                    .unwrap_or_default(),
                total_pages: Some(self.pages.len() as u32),
            })
        }
        fn details(&mut self, _: &str) -> Result<ExtGame, ProviderError> {
            Err(ProviderError::Unavailable("no".into()))
        }
    }

    fn game(id: &str, version: &str, tags: &[&str]) -> ExtGame {
        ExtGame {
            source: "fake".into(),
            ext_id: id.into(),
            title: format!("Game {id}"),
            version: version.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn refresh_is_incremental_and_stops_when_refused() {
        let path = std::env::temp_dir().join(format!("ext-test-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let conn = store::open(&path).unwrap();
        let mut p = Fake {
            pages: vec![
                vec![game("1", "1", &[]), game("2", "1", &["loli"])],
                vec![game("3", "1", &[])],
            ],
            fail_at: None,
            calls: 0,
        };
        let r = refresh(&mut p, &conn, true, 10, &|_| {}).unwrap();
        assert_eq!((r.pages, r.new, r.hidden_by_policy), (2, 2, 1));
        // nothing changed: the incremental run reads one page only
        p.calls = 0;
        let r = refresh(&mut p, &conn, false, 10, &|_| {}).unwrap();
        assert_eq!((p.calls, r.new, r.updated), (1, 0, 0));
        // a new version on page 1 is picked up
        p.pages[0][0].version = "2".into();
        let r = refresh(&mut p, &conn, false, 10, &|_| {}).unwrap();
        assert_eq!(r.updated, 1);
        // the site refuses: the import stops and says why, data read so far stays
        p.fail_at = Some(2);
        let r = refresh(&mut p, &conn, true, 10, &|_| {}).unwrap();
        assert!(r.stopped.as_deref().unwrap().contains("captcha"));
        assert_eq!(store::stats(&conn, "fake").unwrap().games, 2);
        assert!(store::stats(&conn, "fake")
            .unwrap()
            .last_error
            .unwrap()
            .contains("captcha"));
        drop(conn);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn forbidden_tags() {
        assert!(is_forbidden(&["3dcg".into(), "Loli".into()]));
        assert!(!is_forbidden(&["3dcg".into(), "milf".into()]));
    }
}
