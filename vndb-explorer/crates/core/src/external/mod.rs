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
    /// Entries that could not be read (no id...)
    pub bad_items: usize,
}

/// A page of the provider's own index of all games (for the completeness check).
#[derive(Debug, Clone, Default)]
pub struct IndexPage {
    /// Partial records: id, title, maybe developer / version / status / engine; no tags
    pub games: Vec<ExtGame>,
    pub last_page: Option<u32>,
    /// Pinned service threads (rules, announcements) left out
    pub service: usize,
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
    /// One page (1-based) of the catalog feed, newest updates first.
    fn list(&mut self, page: u32) -> Result<ListPage, ProviderError>;
    /// Full metadata of one game from its page (description, official links...).
    fn details(&mut self, ext_id: &str) -> Result<ExtGame, ProviderError>;
    /// One page (1-based) of an independent index of every game, if the site has one.
    fn index_page(&mut self, _page: u32) -> Result<IndexPage, ProviderError> {
        Err(ProviderError::Parse("нет независимого списка игр".into()))
    }
}

/// What an import did. Shown after every run and kept as `last_report`.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct RefreshReport {
    /// "full" (every page of the feed) or "update" (newest pages until nothing changes)
    pub mode: String,
    /// The full pass continued an interrupted one from this page
    pub resumed_from: Option<u32>,
    /// Pages the feed has, by its own count
    pub pages_total: Option<u32>,
    pub pages_read: u32,
    pub pages_failed: Vec<u32>,
    /// Different releases (threads) seen in this run
    pub discovered: usize,
    pub new: usize,
    pub updated: usize,
    /// Seen, nothing changed
    pub unchanged: usize,
    /// Stored but not shown (content policy), by reason
    pub hidden_by_policy: usize,
    /// Entries that could not be read + pages that failed
    pub errors: usize,
    /// The same release met twice in one run (the feed moved while being read)
    pub repeats: usize,
    /// Releases sharing a title and developer with another one in the catalog
    pub duplicates: usize,
    /// Releases merged with a VNDB entry
    pub in_vndb: Option<usize>,
    /// Every page of the feed was read without a gap
    pub complete: bool,
    /// Set when the site stopped the import (rate limit, captcha, login wall...)
    pub stopped: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

/// Retry a failed request once after a pause; refusals by the site are never retried.
fn with_retry<T>(mut f: impl FnMut() -> Result<T, ProviderError>) -> Result<T, ProviderError> {
    match f() {
        Err(ProviderError::Unavailable(_)) => {
            std::thread::sleep(std::time::Duration::from_secs(5));
            f()
        }
        other => other,
    }
}

/// Read the provider's feed, newest updates first.
///
/// * `full`: every page, as many as the feed says it has (`max_pages` is only a safety limit).
///   An interrupted full pass continues where it stopped next time; at the end the first
///   pages are read again until nothing changes, to catch releases that moved up meanwhile.
/// * otherwise ("update"): newest pages until two pages in a row bring nothing new or changed.
///   Allowed only after a full pass finished; until then an update runs as a full pass,
///   so releases missing from the catalog are never skipped.
pub fn refresh(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
    full: bool,
    max_pages: u32,
    log: &dyn Fn(&str),
) -> anyhow::Result<RefreshReport> {
    let src = provider.id();
    let name = provider.name();
    let complete_before = store::get_meta(conn, src, "feed_complete_at")?.is_some();
    let full = full || !complete_before;
    if !full {
        log(&format!("{name}: обновление — читаю новые страницы ленты"));
    } else if !complete_before {
        log(&format!(
            "{name}: полного прохода ещё не было — читаю всю ленту"
        ));
    }
    let mut rep = RefreshReport {
        mode: if full { "full" } else { "update" }.into(),
        started_at: store::stamp(),
        ..Default::default()
    };
    let mut seen = std::collections::HashSet::new();
    let finish = |rep: &mut RefreshReport| -> anyhow::Result<()> {
        rep.duplicates = store::duplicate_count(conn, src)?;
        rep.finished_at = store::stamp();
        store::set_meta(conn, src, "last_report", &serde_json::to_string(rep)?)?;
        store::set_meta(conn, src, "listed_at", &store::stamp())?;
        store::set_meta(
            conn,
            src,
            "last_error",
            rep.stopped.as_deref().unwrap_or(""),
        )?;
        Ok(())
    };
    if let Err(e) = provider.prepare() {
        rep.stopped = Some(e.to_string());
        log(&format!("{name}: {e}"));
        finish(&mut rep)?;
        return Ok(rep);
    }
    // pages: a full pass may continue from where an interrupted one stopped
    let mut page = 1;
    if full {
        if let Some(p) =
            store::get_meta(conn, src, "feed_resume_page")?.and_then(|p| p.parse().ok())
        {
            if p > 1 {
                page = p;
                rep.resumed_from = Some(p);
                log(&format!(
                    "{name}: продолжаю прерванную загрузку со страницы {p}"
                ));
            }
        }
    }
    let mut quiet_pages = 0;
    let mut topping_up = false; // full pass: re-reading the first pages at the end
    loop {
        if page > max_pages {
            log(&format!("{name}: достигнут предел {max_pages} страниц"));
            break;
        }
        let list = match with_retry(|| provider.list(page)) {
            Ok(l) => l,
            Err(e @ ProviderError::Blocked(_)) => {
                rep.stopped = Some(e.to_string());
                log(&format!("{name}: {e}"));
                break;
            }
            Err(e) => {
                rep.pages_failed.push(page);
                rep.errors += 1;
                log(&format!("{name}: страница {page} не прочитана: {e}"));
                if !full {
                    break;
                }
                page += 1;
                continue;
            }
        };
        if let Some(t) = list.total_pages {
            rep.pages_total = Some(t);
        }
        rep.pages_read += 1;
        rep.errors += list.bad_items;
        let mut changed = 0;
        let tx = conn.unchecked_transaction()?;
        for g in &list.games {
            let key = (g.source.clone(), g.ext_id.clone());
            let again = !seen.insert(key);
            if again && !topping_up {
                // the feed moved while being read: the same release on two pages
                rep.repeats += 1;
                continue;
            }
            match store::upsert_listed(&tx, g, "feed")? {
                // read again at the end of a full pass: only real changes count
                store::Change::Unchanged | store::Change::Hidden(_) if again => {}
                store::Change::New | store::Change::Updated if again => {
                    rep.updated += 1;
                    changed += 1;
                }
                store::Change::New => {
                    rep.new += 1;
                    changed += 1;
                }
                store::Change::Updated => {
                    rep.updated += 1;
                    changed += 1;
                }
                store::Change::Unchanged => rep.unchanged += 1,
                store::Change::Hidden(_) => rep.hidden_by_policy += 1,
            }
        }
        tx.commit()?;
        rep.discovered = seen.len();
        log(&format!(
            "{name}: страница {page}{} — найдено {}, новых {}, обновлённых {}",
            rep.pages_total
                .map(|t| format!(" из {t}"))
                .unwrap_or_default(),
            rep.discovered,
            rep.new,
            rep.updated
        ));
        quiet_pages = if changed == 0 { quiet_pages + 1 } else { 0 };
        let last = list.games.is_empty() || rep.pages_total.is_some_and(|t| page >= t);
        if !full || topping_up {
            if quiet_pages >= 2 || last {
                break;
            }
            page += 1;
            continue;
        }
        store::set_meta(conn, src, "feed_resume_page", &(page + 1).to_string())?;
        if last {
            // the whole feed is read; releases that moved to the top meanwhile are caught by
            // reading the first pages again until two of them bring nothing
            rep.complete = rep.pages_failed.is_empty();
            if rep.resumed_from.is_some() || rep.pages_read > 2 {
                topping_up = true;
                quiet_pages = 0;
                page = 1;
                continue;
            }
            break;
        }
        page += 1;
    }
    if full && rep.complete && rep.stopped.is_none() {
        store::del_meta(conn, src, "feed_resume_page")?;
        store::set_meta(conn, src, "feed_complete_at", &store::stamp())?;
    }
    finish(&mut rep)?;
    Ok(rep)
}

/// What the comparison with the site's own index found.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
pub struct AuditReport {
    pub pages_total: Option<u32>,
    pub pages_read: u32,
    pub pages_failed: Vec<u32>,
    /// Different threads in the site's index
    pub index_threads: usize,
    /// Pinned service threads left out
    pub service_threads: usize,
    /// Index threads already in the catalog before the comparison
    pub already_in_catalog: usize,
    /// Index threads the feed did not have: added, waiting for their page to be read
    pub missing_added: usize,
    /// Of those, pages read in this run (tags checked, shown if allowed)
    pub checked: usize,
    pub hidden_by_policy: usize,
    pub errors: usize,
    /// Every index page was read
    pub complete: bool,
    pub stopped: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

/// Compare the catalog with the provider's own index of every game (for F95Zone: the list of
/// threads of the Games forum). Threads the feed never showed are added and their pages are
/// read (up to `check_limit` per run) so their tags can be checked before they are shown.
pub fn audit(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
    max_pages: u32,
    check_limit: usize,
    log: &dyn Fn(&str),
) -> anyhow::Result<AuditReport> {
    let src = provider.id();
    let name = provider.name();
    let mut rep = AuditReport {
        started_at: store::stamp(),
        ..Default::default()
    };
    let known = store::all_ids(conn, src)?;
    let mut seen = std::collections::HashSet::new();
    let mut page = 1;
    while page <= max_pages {
        let idx = match with_retry(|| provider.index_page(page)) {
            Ok(i) => i,
            Err(e @ ProviderError::Blocked(_)) => {
                rep.stopped = Some(e.to_string());
                log(&format!("{name}: {e}"));
                break;
            }
            Err(e) => {
                rep.pages_failed.push(page);
                rep.errors += 1;
                log(&format!("{name}: страница списка {page} не прочитана: {e}"));
                page += 1;
                continue;
            }
        };
        if let Some(t) = idx.last_page {
            rep.pages_total = Some(t);
        }
        rep.pages_read += 1;
        rep.service_threads = rep.service_threads.max(idx.service);
        let tx = conn.unchecked_transaction()?;
        for g in &idx.games {
            if !seen.insert(g.ext_id.clone()) {
                continue;
            }
            if known.contains(&g.ext_id) {
                rep.already_in_catalog += 1;
                continue;
            }
            match store::upsert_listed(&tx, g, "forum")? {
                store::Change::Hidden(_) => rep.hidden_by_policy += 1,
                _ => rep.missing_added += 1,
            }
        }
        tx.commit()?;
        rep.index_threads = seen.len();
        log(&format!(
            "{name}: список тем, страница {page}{} — тем {}, нет в каталоге {}",
            rep.pages_total
                .map(|t| format!(" из {t}"))
                .unwrap_or_default(),
            rep.index_threads,
            rep.missing_added
        ));
        if idx.games.is_empty() || rep.pages_total.is_some_and(|t| page >= t) {
            rep.complete = rep.pages_failed.is_empty();
            break;
        }
        page += 1;
    }
    // read the pages of threads found only in the index, so their tags are known
    if rep.stopped.is_none() {
        for id in store::unchecked(conn, src, check_limit)? {
            match provider.details(&id) {
                Ok(g) => {
                    if let store::Change::Hidden(_) = store::save_details(conn, &g)? {
                        rep.hidden_by_policy += 1;
                    }
                    rep.checked += 1;
                }
                Err(e @ ProviderError::Blocked(_)) => {
                    rep.stopped = Some(e.to_string());
                    log(&format!("{name}: {e}"));
                    break;
                }
                Err(e) => {
                    rep.errors += 1;
                    log(&format!("{name}: тема {id} не прочитана: {e}"));
                }
            }
        }
    }
    rep.finished_at = store::stamp();
    store::set_meta(conn, src, "last_audit", &serde_json::to_string(&rep)?)?;
    Ok(rep)
}

/// Tags that put a game outside this app's content policy (sexual content involving minors).
/// Such games are stored (for the import report) but never shown, see [`store`].
pub const FORBIDDEN_TAGS: &[&str] = &["loli", "shota"];

/// The forbidden tag a game carries, if any.
pub fn policy_reason(tags: &[String]) -> Option<String> {
    tags.iter()
        .map(|t| t.trim().to_lowercase())
        .find(|t| FORBIDDEN_TAGS.contains(&t.as_str()))
}

pub fn is_forbidden(tags: &[String]) -> bool {
    policy_reason(tags).is_some()
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

    /// A feed of `pages`; `index` is the site's own list of every game.
    struct Fake {
        pages: Vec<Vec<ExtGame>>,
        index: Vec<Vec<ExtGame>>,
        fail_at: Option<u32>,
        blocked_at: Option<u32>,
        calls: Vec<u32>,
    }

    impl Fake {
        fn new(pages: Vec<Vec<ExtGame>>) -> Self {
            Fake {
                pages,
                index: Vec::new(),
                fail_at: None,
                blocked_at: None,
                calls: Vec::new(),
            }
        }
    }

    impl Provider for Fake {
        fn id(&self) -> &'static str {
            "fake"
        }
        fn name(&self) -> &'static str {
            "Fake"
        }
        fn list(&mut self, page: u32) -> Result<ListPage, ProviderError> {
            self.calls.push(page);
            if self.blocked_at == Some(page) {
                return Err(ProviderError::Blocked("captcha".into()));
            }
            if self.fail_at == Some(page) {
                return Err(ProviderError::Parse("broken page".into()));
            }
            Ok(ListPage {
                games: self
                    .pages
                    .get(page as usize - 1)
                    .cloned()
                    .unwrap_or_default(),
                total_pages: Some(self.pages.len() as u32),
                bad_items: 0,
            })
        }
        fn details(&mut self, id: &str) -> Result<ExtGame, ProviderError> {
            let mut g = game(id, "1", &["2dcg"]);
            g.description = Some("page".into());
            Ok(g)
        }
        fn index_page(&mut self, page: u32) -> Result<IndexPage, ProviderError> {
            Ok(IndexPage {
                games: self
                    .index
                    .get(page as usize - 1)
                    .cloned()
                    .unwrap_or_default(),
                last_page: Some(self.index.len() as u32),
                service: 1,
            })
        }
    }

    fn game(id: &str, version: &str, tags: &[&str]) -> ExtGame {
        ExtGame {
            source: "fake".into(),
            ext_id: id.into(),
            title: format!("Game {id}"),
            developer: "Dev".into(),
            version: version.into(),
            tags: tags.iter().map(|t| t.to_string()).collect(),
            ..Default::default()
        }
    }

    fn pages(n: usize, per: usize) -> Vec<Vec<ExtGame>> {
        (0..n)
            .map(|p| {
                (0..per)
                    .map(|i| game(&format!("{}", p * per + i + 1), "1", &["x"]))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn full_pass_reads_every_page_the_feed_has() {
        let conn = store::open_memory();
        // far more pages than the old limit of 400
        let mut p = Fake::new(pages(450, 2));
        p.pages[3][0].tags = vec!["loli".into()];
        let r = refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        assert!(r.complete, "{r:?}");
        assert_eq!(r.pages_total, Some(450));
        assert_eq!((r.discovered, r.new, r.hidden_by_policy), (900, 899, 1));
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 900);
        assert!(store::get_meta(&conn, "fake", "feed_complete_at")
            .unwrap()
            .is_some());
        // the report is kept
        let saved: RefreshReport = serde_json::from_str(
            &store::get_meta(&conn, "fake", "last_report")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(saved.discovered, 900);
    }

    #[test]
    fn update_without_a_finished_full_pass_reads_everything() {
        let conn = store::open_memory();
        // a few recent releases are known, the older ones are not
        let mut p = Fake::new(pages(5, 3));
        for g in &p.pages[0] {
            store::upsert_listed(&conn, g, "feed").unwrap();
        }
        let r = refresh(&mut p, &conn, false, 5000, &|_| {}).unwrap();
        assert_eq!(r.mode, "full");
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 15);
        assert!(r.complete);
    }

    #[test]
    fn update_stops_after_two_quiet_pages_and_catches_changes() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(10, 3));
        refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        // nothing changed: two pages are read
        p.calls.clear();
        let r = refresh(&mut p, &conn, false, 5000, &|_| {}).unwrap();
        assert_eq!((r.mode.as_str(), p.calls.len(), r.new), ("update", 2, 0));
        // an old release got a new version and moved to the top; a new one appeared
        let mut moved = p.pages[7].remove(1);
        moved.version = "2".into();
        p.pages[0].insert(0, moved);
        p.pages[0].insert(0, game("999", "1", &["x"]));
        p.calls.clear();
        let r = refresh(&mut p, &conn, false, 5000, &|_| {}).unwrap();
        assert_eq!((r.new, r.updated), (1, 1));
        assert_eq!(p.calls, vec![1, 2, 3]);
    }

    #[test]
    fn interrupted_full_pass_resumes_and_failed_pages_are_reported() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(6, 2));
        p.blocked_at = Some(4);
        let r = refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        assert!(r.stopped.as_deref().unwrap().contains("captcha"));
        assert!(!r.complete);
        assert_eq!(r.discovered, 6);
        assert!(store::get_meta(&conn, "fake", "feed_complete_at")
            .unwrap()
            .is_none());
        // next time: continues at page 4, then reads the top again
        p.blocked_at = None;
        p.fail_at = Some(5);
        p.calls.clear();
        let r = refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        assert_eq!(r.resumed_from, Some(4));
        assert_eq!(&p.calls[..3], &[4, 5, 6]);
        assert_eq!(r.pages_failed, vec![5]);
        assert_eq!(r.errors, 1);
        // a gap: not complete, so an update still runs as a full pass
        assert!(!r.complete);
        assert!(store::get_meta(&conn, "fake", "feed_complete_at")
            .unwrap()
            .is_none());
    }

    #[test]
    fn repeats_are_counted_not_stored_twice() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(3, 2));
        // the feed shifted while being read: page 2 starts with the last entry of page 1
        let dup = p.pages[0][1].clone();
        p.pages[1].insert(0, dup);
        let r = refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        assert_eq!((r.discovered, r.repeats), (6, 1));
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 6);
    }

    #[test]
    fn audit_adds_what_the_feed_missed() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(2, 2));
        refresh(&mut p, &conn, true, 5000, &|_| {}).unwrap();
        // the forum knows two more threads
        let mut extra = game("50", "1", &[]);
        extra.tags.clear();
        p.index = vec![
            p.pages[0].clone(),
            vec![p.pages[1][0].clone(), extra.clone(), game("51", "1", &[])],
        ];
        let r = audit(&mut p, &conn, 5000, 100, &|_| {}).unwrap();
        assert!(r.complete);
        assert_eq!(
            (
                r.index_threads,
                r.already_in_catalog,
                r.missing_added,
                r.checked
            ),
            (5, 3, 2, 2)
        );
        // their pages were read, so they are shown now
        assert!(store::get(&conn, "fake", "50").unwrap().is_some());
        assert_eq!(store::stats(&conn, "fake").unwrap().games, 6);
    }

    #[test]
    fn forbidden_tags() {
        assert!(is_forbidden(&["3dcg".into(), "Loli".into()]));
        assert_eq!(policy_reason(&["Shota".into()]).as_deref(), Some("shota"));
        assert!(!is_forbidden(&["3dcg".into(), "milf".into()]));
    }
}
