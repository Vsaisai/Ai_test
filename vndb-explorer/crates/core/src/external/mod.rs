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
    /// The site asked to slow down ("too many requests"): wait this many seconds, then go on
    /// slower. This is the site's own rule, so the app keeps to it.
    RateLimited { wait_secs: u64 },
    /// The user pressed Stop.
    Cancelled,
    /// Network error or the site is down; worth trying again later.
    Unavailable(String),
    /// The answer did not have the expected shape (the site changed).
    Parse(String),
}

impl std::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProviderError::Blocked(m) => write!(f, "сайт ограничил доступ: {m}"),
            ProviderError::RateLimited { wait_secs } => write!(
                f,
                "сайт попросил подождать (слишком много запросов), примерно {} мин",
                wait_secs.div_ceil(60).max(1)
            ),
            ProviderError::Unavailable(m) => write!(f, "сайт недоступен: {m}"),
            ProviderError::Parse(m) => write!(f, "не удалось разобрать ответ сайта: {m}"),
            ProviderError::Cancelled => write!(f, "остановлено"),
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

/// What a run reports to and is steered by: a text log, stage progress and the Stop button.
pub struct Run<'a> {
    pub log: &'a dyn Fn(&str),
    /// (stage id, units done, units in the stage if known)
    pub progress: &'a dyn Fn(&'static str, u64, Option<u64>),
    /// true once the user asked to stop: the run ends after the current request
    pub stop: &'a dyn Fn() -> bool,
}

impl Run<'static> {
    /// No log, no progress, never stopped (tests, command line helpers).
    pub fn quiet() -> Self {
        Run {
            log: &|_| {},
            progress: &|_, _, _| {},
            stop: &|| false,
        }
    }
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
    /// The site answered badly (empty page, timeout): ask less often for the rest of the run.
    fn slow_down(&mut self) {}
    /// The site asked for a pause until this moment (unix seconds): no requests before it.
    fn respect_cooldown(&mut self, _until_unix: u64) {}
    /// End of the current pause (unix seconds), to remember it across restarts.
    fn cooldown_until(&self) -> Option<u64> {
        None
    }
    /// Wait out a "too many requests" pause; ends early when `stop` says so (tests do not wait).
    fn rate_pause(&mut self, secs: u64, stop: &dyn Fn() -> bool) {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(secs);
        while std::time::Instant::now() < until && !stop() {
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
    }
    /// Pause before a request is repeated.
    fn retry_pause(&self) -> std::time::Duration {
        std::time::Duration::from_secs(10)
    }
    /// One page (1-based) of an independent index of every game, if the site has one.
    fn index_page(&mut self, _page: u32) -> Result<IndexPage, ProviderError> {
        Err(ProviderError::Parse("нет независимого списка игр".into()))
    }
}

/// What an import did. Shown after every run and kept as `last_report`.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
#[serde(default)]
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
    /// The user pressed Stop (the next run continues from the same page)
    pub cancelled: bool,
    pub started_at: String,
    pub finished_at: String,
}

/// Retry a failed request once after a pause; refusals by the site are never retried.
/// Apply a pause saved by an earlier run (the app may have been restarted meanwhile).
pub fn load_cooldown(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
) -> anyhow::Result<()> {
    if let Some(t) =
        store::get_meta(conn, provider.id(), "cooldown_until")?.and_then(|t| t.parse().ok())
    {
        provider.respect_cooldown(t);
    }
    Ok(())
}

/// Remember the provider's pause, if any.
pub fn save_cooldown(provider: &dyn Provider, conn: &rusqlite::Connection) -> anyhow::Result<()> {
    if let Some(t) = provider.cooldown_until() {
        store::set_meta(conn, provider.id(), "cooldown_until", &t.to_string())?;
    }
    Ok(())
}

/// How many times one run waits out "too many requests" before it stops (and resumes next time).
pub const MAX_RATE_WAITS: u32 = 3;

/// Repeat a request after a temporary failure (empty answer, timeout, server error): up to two
/// more tries, with a pause, and the provider slows down for the rest of the run. Refusals by
/// the site (rate limit, captcha, login) are never repeated.
fn with_retry<T>(
    provider: &mut dyn Provider,
    run: &Run,
    mut f: impl FnMut(&mut dyn Provider) -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    let log = run.log;
    let mut tries = 0;
    let mut waits = 0;
    loop {
        if (run.stop)() {
            return Err(ProviderError::Cancelled);
        }
        // a bug while reading one page must not end the whole run: it counts as that page's error
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(&mut *provider)))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_default();
                Err(ProviderError::Parse(format!(
                    "ошибка разбора страницы: {msg}"
                )))
            });
        match res {
            // the site asked to slow down: wait as long as it said, then go on twice as slowly
            Err(ProviderError::RateLimited { wait_secs }) if waits < MAX_RATE_WAITS => {
                waits += 1;
                provider.slow_down();
                provider.slow_down();
                log(&format!(
                    "{}: сайт попросил подождать — пауза {} мин, затем продолжу медленнее ({} из {MAX_RATE_WAITS})",
                    provider.name(),
                    wait_secs.div_ceil(60).max(1),
                    waits
                ));
                provider.rate_pause(wait_secs, run.stop);
            }
            Err(ProviderError::Unavailable(m)) if tries < 2 => {
                tries += 1;
                provider.slow_down();
                let pause = provider.retry_pause() * tries;
                log(&format!(
                    "{}: {m} — повтор через {} с",
                    provider.name(),
                    pause.as_secs()
                ));
                std::thread::sleep(pause);
            }
            other => return other,
        }
    }
}

/// Save one page of the feed; returns how many releases are new or changed.
fn ingest(
    conn: &rusqlite::Connection,
    games: &[ExtGame],
    seen: &mut std::collections::HashSet<(String, String)>,
    reread: bool,
    rep: &mut RefreshReport,
) -> anyhow::Result<usize> {
    let mut changed = 0;
    let tx = conn.unchecked_transaction()?;
    for g in games {
        let key = (g.source.clone(), g.ext_id.clone());
        let again = !seen.insert(key);
        if again && !reread {
            // the feed moved while being read: the same release on two pages
            rep.repeats += 1;
            continue;
        }
        match store::upsert_listed(&tx, g, "feed")? {
            // read again (end of a full pass, a retried page): only real changes count
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
    Ok(changed)
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
    run: &Run,
) -> anyhow::Result<RefreshReport> {
    let log = run.log;
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
    load_cooldown(provider, conn)?;
    let finish = |rep: &mut RefreshReport| -> anyhow::Result<()> {
        rep.cancelled = (run.stop)();
        if rep.cancelled {
            // not the site's doing: nothing to warn about, the next run continues
            rep.stopped = None;
            rep.complete = false;
        }
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
    if let Err(e) = with_retry(provider, run, |p| p.prepare()) {
        rep.stopped = Some(e.to_string());
        log(&format!("{name}: {e}"));
        save_cooldown(provider, conn)?;
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
        let list = match with_retry(provider, run, |p| p.list(page)) {
            Ok(l) => l,
            Err(
                e @ (ProviderError::Blocked(_)
                | ProviderError::RateLimited { .. }
                | ProviderError::Cancelled),
            ) => {
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
        // pages read again at the end of a full pass are not new pages of the feed
        if !topping_up {
            rep.pages_read += 1;
        }
        rep.errors += list.bad_items;
        let changed = ingest(conn, &list.games, &mut seen, topping_up, &mut rep)?;
        if !topping_up {
            (run.progress)("external", page as u64, rep.pages_total.map(u64::from));
        }
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
            // pages that failed during the pass get one more chance now
            if !rep.pages_failed.is_empty() {
                let failed = std::mem::take(&mut rep.pages_failed);
                log(&format!(
                    "{name}: повторно читаю непрочитанные страницы: {failed:?}"
                ));
                for fp in failed {
                    match with_retry(provider, run, |p| p.list(fp)) {
                        Ok(l) => {
                            rep.pages_read += 1;
                            rep.errors = rep.errors.saturating_sub(1) + l.bad_items;
                            ingest(conn, &l.games, &mut seen, true, &mut rep)?;
                            log(&format!(
                                "{name}: страница {fp} прочитана со второй попытки"
                            ));
                        }
                        Err(
                            e @ (ProviderError::Blocked(_)
                            | ProviderError::RateLimited { .. }
                            | ProviderError::Cancelled),
                        ) => {
                            rep.pages_failed.push(fp);
                            rep.stopped = Some(e.to_string());
                            log(&format!("{name}: {e}"));
                            break;
                        }
                        Err(e) => {
                            rep.pages_failed.push(fp);
                            log(&format!("{name}: страница {fp} снова не прочитана: {e}"));
                        }
                    }
                }
                if rep.stopped.is_some() {
                    break;
                }
            }
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
    if full && rep.complete && rep.stopped.is_none() && !(run.stop)() {
        store::del_meta(conn, src, "feed_resume_page")?;
        store::set_meta(conn, src, "feed_complete_at", &store::stamp())?;
    }
    save_cooldown(provider, conn)?;
    finish(&mut rep)?;
    Ok(rep)
}

/// What the comparison with the site's own index found.
#[derive(Debug, Clone, Default, Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AuditReport {
    /// An interrupted comparison continued from this page (counts include the earlier part)
    pub resumed_from: Option<u32>,
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
    /// The index walk was reused from a recent complete run; only thread pages were read
    pub index_skipped: bool,
    /// Threads still waiting for their page to be read (the next run goes on with them)
    pub pending: usize,
    /// The user pressed Stop
    pub cancelled: bool,
    pub stopped: Option<String>,
    pub started_at: String,
    pub finished_at: String,
}

/// A finished walk over the index is reused for this long: a second run only reads the pages
/// of threads still waiting for their tag check.
pub const INDEX_FRESH_SECS: u64 = 24 * 3600;

/// Compare the catalog with the provider's own index of every game (for F95Zone: the list of
/// threads of the Games forum), in two stages:
///
/// 1. `external-audit` — walk the index; threads the feed never showed are added. An interrupted
///    walk continues from its page. Skipped when a complete walk finished less than
///    [`INDEX_FRESH_SECS`] ago and threads are still waiting for stage 2.
/// 2. `external-pages` — read the page of every waiting thread (up to `check_limit`), so its
///    tags can be checked before it is shown. What is left is read by the next run.
pub fn audit(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
    max_pages: u32,
    check_limit: usize,
    run: &Run,
) -> anyhow::Result<AuditReport> {
    let log = run.log;
    let src = provider.id();
    let name = provider.name();
    let mut rep = AuditReport {
        started_at: store::stamp(),
        ..Default::default()
    };
    load_cooldown(provider, conn)?;
    let resume = store::get_meta(conn, src, "audit_resume")?
        .and_then(|j| serde_json::from_str::<(u32, AuditReport)>(&j).ok())
        .filter(|(p, _)| *p > 1);
    let last = store::get_meta(conn, src, "last_audit")?
        .and_then(|j| serde_json::from_str::<AuditReport>(&j).ok());
    let waiting = store::unchecked_count(conn, src)?;
    let fresh = last.as_ref().filter(|l| {
        l.complete
            && resume.is_none()
            && waiting > 0
            && store::age_secs(&Some(l.finished_at.clone())).is_some_and(|a| a < INDEX_FRESH_SECS)
    });
    if let Some(l) = fresh {
        // stage 1 was finished recently: only the waiting pages are left
        log(&format!(
            "{name}: список тем уже пройден — дочитываю страницы тем ({waiting})"
        ));
        rep = AuditReport {
            index_skipped: true,
            resumed_from: None,
            stopped: None,
            cancelled: false,
            ..l.clone()
        };
    } else {
        walk_index(provider, conn, max_pages, run, &mut rep, resume)?;
    }
    // stage 2: pages of threads found only in the index, so their tags are known
    if rep.stopped.is_none() && !(run.stop)() {
        let ids = store::unchecked(conn, src, check_limit)?;
        let total = ids.len() as u64;
        if total > 0 {
            log(&format!("{name}: читаю страницы новых тем — {total}"));
            (run.progress)("external-pages", 0, Some(total));
        }
        for (i, id) in ids.iter().enumerate() {
            match with_retry(provider, run, |p| p.details(id)) {
                Ok(g) => {
                    let hidden = matches!(store::save_details(conn, &g)?, store::Change::Hidden(_));
                    if hidden {
                        rep.hidden_by_policy += 1;
                    }
                    rep.checked += 1;
                    log(&format!(
                        "{name}: тема {} из {total} — «{}»{}",
                        i + 1,
                        g.title,
                        if hidden {
                            " (скрыта правилами)"
                        } else {
                            ""
                        }
                    ));
                }
                Err(
                    e @ (ProviderError::Blocked(_)
                    | ProviderError::RateLimited { .. }
                    | ProviderError::Cancelled),
                ) => {
                    rep.stopped = Some(e.to_string());
                    log(&format!("{name}: {e}"));
                    break;
                }
                Err(e) => {
                    rep.errors += 1;
                    // a deleted or broken thread must not be asked for again and again
                    store::mark_check_failed(conn, src, id)?;
                    log(&format!("{name}: тема {id} не прочитана: {e}"));
                }
            }
            (run.progress)("external-pages", i as u64 + 1, Some(total));
        }
    }
    rep.pending = store::unchecked_count(conn, src)?;
    rep.cancelled = (run.stop)();
    if rep.cancelled {
        rep.stopped = None;
    }
    rep.finished_at = store::stamp();
    save_cooldown(provider, conn)?;
    store::set_meta(conn, src, "last_audit", &serde_json::to_string(&rep)?)?;
    Ok(rep)
}

/// Stage 1 of [`audit`]: walk the provider's index and add the threads the catalog lacks.
fn walk_index(
    provider: &mut dyn Provider,
    conn: &rusqlite::Connection,
    max_pages: u32,
    run: &Run,
    rep: &mut AuditReport,
    resume: Option<(u32, AuditReport)>,
) -> anyhow::Result<()> {
    let log = run.log;
    let src = provider.id();
    let name = provider.name();
    let known = store::all_ids(conn, src)?;
    let mut seen = std::collections::HashSet::new();
    let mut page = 1;
    // an interrupted walk continues where it stopped, with the counts it had
    let mut earlier_threads = 0;
    if let Some((p, r)) = resume {
        log(&format!("{name}: продолжаю сверку со страницы {p}"));
        earlier_threads = r.index_threads;
        *rep = AuditReport {
            resumed_from: Some(p),
            started_at: r.started_at.clone(),
            stopped: None,
            cancelled: false,
            complete: false,
            index_skipped: false,
            ..r
        };
        page = p;
    }
    let mut add = |games: &[ExtGame], rep: &mut AuditReport| -> anyhow::Result<()> {
        let tx = conn.unchecked_transaction()?;
        for g in games {
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
        rep.index_threads = earlier_threads + seen.len();
        Ok(())
    };
    while page <= max_pages {
        let idx = match with_retry(provider, run, |p| p.index_page(page)) {
            Ok(i) => i,
            Err(
                e @ (ProviderError::Blocked(_)
                | ProviderError::RateLimited { .. }
                | ProviderError::Cancelled),
            ) => {
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
        add(&idx.games, rep)?;
        (run.progress)(
            "external-audit",
            page as u64,
            rep.pages_total.map(u64::from),
        );
        store::set_meta(
            conn,
            src,
            "audit_resume",
            &serde_json::to_string(&(page + 1, &*rep))?,
        )?;
        log(&format!(
            "{name}: список тем, страница {page}{} — тем {}, нет в каталоге {}",
            rep.pages_total
                .map(|t| format!(" из {t}"))
                .unwrap_or_default(),
            rep.index_threads,
            rep.missing_added
        ));
        if idx.games.is_empty() || rep.pages_total.is_some_and(|t| page >= t) {
            // pages that failed get one more chance
            for fp in std::mem::take(&mut rep.pages_failed) {
                match with_retry(provider, run, |p| p.index_page(fp)) {
                    Ok(i) => {
                        rep.pages_read += 1;
                        rep.errors = rep.errors.saturating_sub(1);
                        add(&i.games, rep)?;
                    }
                    Err(_) => rep.pages_failed.push(fp),
                }
            }
            rep.complete = rep.pages_failed.is_empty() && !(run.stop)();
            break;
        }
        page += 1;
    }
    if rep.complete {
        store::del_meta(conn, src, "audit_resume")?;
    }
    Ok(())
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
        /// page -> how many more times it answers "temporarily unavailable"
        flaky: std::collections::HashMap<u32, u32>,
        /// page -> how many more times it answers "too many requests"
        limited: std::collections::HashMap<u32, u32>,
        paused: Vec<u64>,
        index_limit_at: Option<u32>,
        index_calls: u32,
        calls: Vec<u32>,
        slowed: u32,
    }

    impl Fake {
        fn new(pages: Vec<Vec<ExtGame>>) -> Self {
            Fake {
                pages,
                index: Vec::new(),
                fail_at: None,
                blocked_at: None,
                flaky: Default::default(),
                limited: Default::default(),
                paused: Vec::new(),
                index_limit_at: None,
                index_calls: 0,
                calls: Vec::new(),
                slowed: 0,
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
            if let Some(n) = self.limited.get_mut(&page).filter(|n| **n > 0) {
                *n -= 1;
                return Err(ProviderError::RateLimited { wait_secs: 900 });
            }
            if let Some(n) = self.flaky.get_mut(&page).filter(|n| **n > 0) {
                *n -= 1;
                return Err(ProviderError::Unavailable("пустой ответ".into()));
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
        fn slow_down(&mut self) {
            self.slowed += 1;
        }
        fn retry_pause(&self) -> std::time::Duration {
            std::time::Duration::ZERO
        }
        fn rate_pause(&mut self, secs: u64, _stop: &dyn Fn() -> bool) {
            self.paused.push(secs);
        }
        fn details(&mut self, id: &str) -> Result<ExtGame, ProviderError> {
            if id == "66" {
                panic!("byte index 12 is not a char boundary");
            }
            let mut g = game(id, "1", &["2dcg"]);
            g.description = Some("page".into());
            Ok(g)
        }
        fn index_page(&mut self, page: u32) -> Result<IndexPage, ProviderError> {
            self.index_calls += 1;
            if self.index_limit_at == Some(page) {
                return Err(ProviderError::Blocked("captcha".into()));
            }
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
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
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
        let r = refresh(&mut p, &conn, false, 5000, &Run::quiet()).unwrap();
        assert_eq!(r.mode, "full");
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 15);
        assert!(r.complete);
    }

    #[test]
    fn update_stops_after_two_quiet_pages_and_catches_changes() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(10, 3));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        // nothing changed: two pages are read
        p.calls.clear();
        let r = refresh(&mut p, &conn, false, 5000, &Run::quiet()).unwrap();
        assert_eq!((r.mode.as_str(), p.calls.len(), r.new), ("update", 2, 0));
        // an old release got a new version and moved to the top; a new one appeared
        let mut moved = p.pages[7].remove(1);
        moved.version = "2".into();
        p.pages[0].insert(0, moved);
        p.pages[0].insert(0, game("999", "1", &["x"]));
        p.calls.clear();
        let r = refresh(&mut p, &conn, false, 5000, &Run::quiet()).unwrap();
        assert_eq!((r.new, r.updated), (1, 1));
        assert_eq!(p.calls, vec![1, 2, 3]);
    }

    #[test]
    fn interrupted_full_pass_resumes_and_failed_pages_are_reported() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(6, 2));
        p.blocked_at = Some(4);
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
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
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
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
    fn temporary_failures_are_retried_and_recovered() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(5, 2));
        // page 2: two empty answers, the third try works
        p.flaky.insert(2, 2);
        // page 4: fails all three tries during the pass, works when retried at the end
        p.flaky.insert(4, 3);
        let progress = std::cell::RefCell::new(Vec::new());
        let run = Run {
            progress: &|stage, d, t| {
                assert_eq!(stage, "external");
                progress.borrow_mut().push((d, t))
            },
            ..Run::quiet()
        };
        let r = refresh(&mut p, &conn, true, 5000, &run).unwrap();
        assert!(r.complete, "{r:?}");
        assert!(r.pages_failed.is_empty());
        assert_eq!(r.errors, 0);
        assert_eq!(r.discovered, 10);
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 10);
        assert!(p.slowed >= 3, "the provider was asked to slow down");
        // page progress for the UI: 1..5 of 5
        assert_eq!(progress.borrow().first(), Some(&(1, Some(5))));
        assert!(progress.borrow().contains(&(5, Some(5))));
    }

    #[test]
    fn too_many_requests_means_wait_and_go_on_slower() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(4, 2));
        p.limited.insert(3, 1);
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        // waited as asked, then read the page and finished
        assert_eq!(p.paused, vec![900]);
        assert!(r.complete && r.stopped.is_none(), "{r:?}");
        assert_eq!(r.discovered, 8);
        assert!(p.slowed >= 2);
    }

    #[test]
    fn a_site_that_keeps_refusing_stops_the_run_for_later() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(4, 2));
        p.limited.insert(2, 99);
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        assert_eq!(p.paused.len() as u32, MAX_RATE_WAITS);
        assert!(r.stopped.as_deref().unwrap().contains("подождать"));
        assert!(!r.complete);
        // next time the full pass continues from page 2
        p.limited.clear();
        p.calls.clear();
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        assert_eq!(r.resumed_from, Some(2));
        assert!(r.complete);
    }

    #[test]
    fn a_crash_on_one_page_is_an_error_not_the_end() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(1, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        p.index = vec![vec![game("66", "1", &[]), game("67", "1", &[])]];
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert!(r.stopped.is_none());
        assert_eq!((r.missing_added, r.checked, r.errors), (2, 1, 1));
        assert!(store::get(&conn, "fake", "67").unwrap().is_some());
    }

    #[test]
    fn interrupted_audit_resumes() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(1, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        p.index = vec![
            vec![game("1", "1", &[]), game("70", "1", &[])],
            vec![game("71", "1", &[])],
            vec![game("72", "1", &[])],
        ];
        // stopped on page 2
        p.index_limit_at = Some(2);
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert!(r.stopped.is_some() && !r.complete);
        assert_eq!(r.missing_added, 1);
        // next time: from page 2, counts carried over
        p.index_limit_at = None;
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert_eq!(r.resumed_from, Some(2));
        assert!(r.complete, "{r:?}");
        assert_eq!((r.index_threads, r.missing_added), (4, 3));
        assert!(store::get_meta(&conn, "fake", "audit_resume")
            .unwrap()
            .is_none());
    }

    #[test]
    fn audit_reports_thread_pages_as_their_own_stage() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(1, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        p.index = vec![vec![
            game("80", "1", &[]),
            game("81", "1", &[]),
            game("82", "1", &[]),
        ]];
        let stages = std::cell::RefCell::new(Vec::new());
        let lines = std::cell::RefCell::new(Vec::new());
        let run = Run {
            log: &|m| lines.borrow_mut().push(m.to_string()),
            progress: &|stage, d, t| stages.borrow_mut().push((stage, d, t)),
            ..Run::quiet()
        };
        let r = audit(&mut p, &conn, 5000, 100, &run).unwrap();
        assert_eq!((r.checked, r.pending), (3, 0));
        let st = stages.borrow();
        assert!(st.contains(&("external-audit", 1, Some(1))));
        // the second stage counts thread pages: 0 of 3 ... 3 of 3
        assert!(st.contains(&("external-pages", 0, Some(3))));
        assert!(st.contains(&("external-pages", 3, Some(3))));
        assert!(lines.borrow().iter().any(|l| l.contains("тема 2 из 3")));
    }

    #[test]
    fn a_second_run_only_reads_the_waiting_pages() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(1, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        p.index = vec![(80..90).map(|i| game(&i.to_string(), "1", &[])).collect()];
        // the first run may read 4 thread pages
        let r = audit(&mut p, &conn, 5000, 4, &Run::quiet()).unwrap();
        assert!(r.complete && !r.index_skipped);
        assert_eq!((r.missing_added, r.checked, r.pending), (10, 4, 6));
        // the second run does not walk the index again
        p.index_calls = 0;
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert!(r.index_skipped && r.complete);
        assert_eq!(p.index_calls, 0);
        assert_eq!(
            (r.index_threads, r.missing_added, r.checked, r.pending),
            (10, 10, 10, 0)
        );
        // nothing is waiting any more: the next run is a real comparison again
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert!(!r.index_skipped);
        assert_eq!(p.index_calls, 1);
    }

    #[test]
    fn stop_ends_the_run_and_the_next_one_continues() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(6, 2));
        let calls = std::cell::Cell::new(0);
        // the user presses Stop while page 3 is being read
        let run = Run {
            progress: &|_, d, _| calls.set(d),
            stop: &|| calls.get() >= 3,
            ..Run::quiet()
        };
        let r = refresh(&mut p, &conn, true, 5000, &run).unwrap();
        assert!(r.cancelled && !r.complete && r.stopped.is_none());
        assert_eq!(p.calls, vec![1, 2, 3]);
        assert_eq!(
            store::get_meta(&conn, "fake", "last_error")
                .unwrap()
                .as_deref(),
            Some("")
        );
        // pressing the button again goes on from page 4
        p.calls.clear();
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        assert_eq!(r.resumed_from, Some(4));
        assert!(r.complete && !r.cancelled);
        // the comparison stops the same way, between thread pages
        p.index = vec![(80..86).map(|i| game(&i.to_string(), "1", &[])).collect()];
        let read = std::cell::Cell::new(0);
        let run = Run {
            progress: &|stage, d, _| {
                if stage == "external-pages" {
                    read.set(d)
                }
            },
            stop: &|| read.get() >= 2,
            ..Run::quiet()
        };
        let r = audit(&mut p, &conn, 5000, 100, &run).unwrap();
        assert!(r.cancelled && r.stopped.is_none());
        assert_eq!((r.checked, r.pending), (2, 4));
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        assert!(r.index_skipped);
        assert_eq!((r.checked, r.pending), (6, 0));
    }

    #[test]
    fn a_thread_that_never_opens_stops_being_asked_for() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(1, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        p.index = vec![vec![game("66", "1", &[]), game("67", "1", &[])]];
        for _ in 0..store::MAX_CHECK_FAILS {
            audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
        }
        let s = store::stats(&conn, "fake").unwrap();
        assert_eq!((s.needs_check, s.check_failed), (0, 1));
        assert!(store::get(&conn, "fake", "66").unwrap().is_none());
    }

    #[test]
    fn repeats_are_counted_not_stored_twice() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(3, 2));
        // the feed shifted while being read: page 2 starts with the last entry of page 1
        let dup = p.pages[0][1].clone();
        p.pages[1].insert(0, dup);
        let r = refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        assert_eq!((r.discovered, r.repeats), (6, 1));
        assert_eq!(store::stats(&conn, "fake").unwrap().stored, 6);
    }

    #[test]
    fn audit_adds_what_the_feed_missed() {
        let conn = store::open_memory();
        let mut p = Fake::new(pages(2, 2));
        refresh(&mut p, &conn, true, 5000, &Run::quiet()).unwrap();
        // the forum knows two more threads
        let mut extra = game("50", "1", &[]);
        extra.tags.clear();
        p.index = vec![
            p.pages[0].clone(),
            vec![p.pages[1][0].clone(), extra.clone(), game("51", "1", &[])],
        ];
        let r = audit(&mut p, &conn, 5000, 100, &Run::quiet()).unwrap();
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
