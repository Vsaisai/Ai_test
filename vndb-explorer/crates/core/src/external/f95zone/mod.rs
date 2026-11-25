//! F95Zone as a catalog of games (metadata only).
//!
//! What is used, and why it is allowed (checked November 2026):
//! * `robots.txt` allows every path used here for `User-agent: *` and states
//!   `Content-Signal: search=yes, ai-train=no` — building a searchable catalog with links and
//!   short excerpts is permitted; nothing here trains AI models.
//! * The site rules (`/help/terms/`) do not forbid automated reading of public pages.
//! * `/sam/latest_alpha/latest_data.php` is the public JSON feed behind the site's own "Latest
//!   updates" page (no login needed): id, title, developer, version, tags, prefixes (engine,
//!   status), rating, likes, views, cover, date.
//! * `/threads/<id>/` — the public game page: description, developer and official links.
//!   Read only when the user opens a game, never in bulk.
//!
//! Rules this provider follows: at least [`MIN_GAP`] between requests, a User-Agent naming the
//! app, no cookies or login, and it **stops** on rate limits, anti-bot checks (Cloudflare,
//! DDoS-Guard), captchas or login walls instead of working around them. Download sections,
//! file hosts and forum redirect links are never read into the catalog.
//!
//! Layout:
//! * [`html`] — small, panic-free XenForo HTML helpers (shared by all parsers)
//! * [`feed`] — the "Latest updates" JSON feed, tag dictionaries, request pace
//! * [`thread`] — the game page and the forum thread list (HTML parsers)
//! * [`check`] — the short polite live check against the real site
//! * this module — the [`F95Zone`] provider itself, cooldowns and re-exports

use super::{ExtGame, ListPage, Provider, ProviderError};
use std::collections::HashMap;
use std::time::Duration;

pub mod feed;
pub mod html;
mod check;
mod thread;
pub use check::{live_check, CheckStep, LiveCheck};
pub use thread::{forum_page as parse_forum_page, game_page as parse_thread};

pub const ID: &str = "f95zone";
pub const HOST: &str = "https://f95zone.to";
pub const MIN_GAP: Duration = Duration::from_secs(3);
/// Pause after "too many requests" when the site does not say how long.
pub const DEFAULT_COOLDOWN: u64 = 15 * 60;

/// Until when the site asked for a pause (shared by the whole app: jobs and opened pages).
static COOLDOWN_UNTIL: std::sync::Mutex<Option<std::time::SystemTime>> =
    std::sync::Mutex::new(None);

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Seconds left of the pause the site asked for (None = no pause).
pub fn cooldown_left() -> Option<u64> {
    let until = (*COOLDOWN_UNTIL.lock().unwrap_or_else(|e| e.into_inner()))?;
    until
        .duration_since(std::time::SystemTime::now())
        .ok()
        .map(|d| d.as_secs().max(1))
}

/// Remember a pause until `until_unix` (from the saved state after a restart).
pub fn set_cooldown_until(until_unix: u64) {
    if until_unix > unix_now() {
        let t = std::time::UNIX_EPOCH + Duration::from_secs(until_unix);
        let mut c = COOLDOWN_UNTIL.lock().unwrap_or_else(|e| e.into_inner());
        if c.is_none_or(|old| old < t) {
            *c = Some(t);
        }
    }
}

/// When the current pause ends (unix seconds), for saving.
pub fn cooldown_until_unix() -> Option<u64> {
    cooldown_left().map(|l| unix_now() + l)
}

pub const PER_PAGE: u32 = 90;
/// Longest description kept (the overview of a game is normally far shorter).
pub const DESCRIPTION_MAX: usize = 20_000;
/// The Games forum: its thread list is the site's own index of every game.
pub const GAMES_FORUM: &str = "/forums/games.2/";

/// Engines recognized from a thread prefix.
const ENGINES: &[&str] = &[
    "Ren'Py",
    "RPGM",
    "Unity",
    "Unreal Engine",
    "HTML",
    "Flash",
    "Java",
    "QSP",
    "RAGS",
    "ADRIFT",
    "Tads",
    "WebGL",
    "Wolf RPG",
    "Godot",
    "Others",
];

/// A prefix named "Completed" / "Onhold" / "Abandoned" gives the release status.
pub(crate) fn status_of(name: &str) -> Option<&'static str> {
    match name.to_lowercase().replace(' ', "").as_str() {
        "completed" => Some("completed"),
        "onhold" => Some("onhold"),
        "abandoned" => Some("abandoned"),
        _ => None,
    }
}

/// An engine prefix keeps its canonical spelling.
pub(crate) fn engine_of(name: &str) -> Option<&'static str> {
    ENGINES
        .iter()
        .find(|e| e.eq_ignore_ascii_case(name.trim()))
        .copied()
}

pub struct F95Zone {
    http: Option<reqwest::blocking::Client>,
    pub min_gap: Duration,
    tags: HashMap<u64, String>,
    prefixes: HashMap<u64, String>,
    /// Where the tag names came from: "site" or "snapshot"
    pub dictionary: &'static str,
    /// Address requests go to: the site, or a local test server (`VNSHELF_F95_BASE`, development
    /// only). Stored page addresses always point to the real site.
    base: String,
}

impl Default for F95Zone {
    fn default() -> Self {
        Self::new()
    }
}

impl F95Zone {
    pub fn new() -> Self {
        F95Zone {
            http: None,
            min_gap: MIN_GAP,
            tags: feed::fallback_tags(),
            prefixes: HashMap::new(),
            dictionary: "snapshot",
            base: std::env::var("VNSHELF_F95_BASE")
                .ok()
                .filter(|b| {
                    b.starts_with("http://127.0.0.1:") || b.starts_with("http://localhost:")
                })
                .unwrap_or_else(|| HOST.to_string()),
        }
    }

    fn client(&mut self) -> Result<&reqwest::blocking::Client, ProviderError> {
        if self.http.is_none() {
            let c = reqwest::blocking::Client::builder()
                .user_agent(format!(
                    "VNShelf/{} (game catalog; +https://github.com/Vsaisai/Ai_test)",
                    crate::app_version()
                ))
                .timeout(Duration::from_secs(30))
                .build()
                .map_err(|e| ProviderError::Unavailable(e.to_string()))?;
            self.http = Some(c);
        }
        Ok(self.http.as_ref().expect("client"))
    }

    fn get(&mut self, url: &str) -> Result<String, ProviderError> {
        // the site asked for a pause: no request until it ends
        if let Some(left) = cooldown_left() {
            return Err(ProviderError::RateLimited { wait_secs: left });
        }
        // one pace for the whole process: the import job and pages opened by the user share it
        feed::pace(self.min_gap);
        let res = self.client()?.get(url).send();
        let res = res.map_err(|e| ProviderError::Unavailable(e.to_string()))?;
        let status = res.status().as_u16();
        // Retry-After in seconds, when the site says how long to wait
        let retry_after = res
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim().parse::<u64>().ok());
        // a connection cut while the body is read is a temporary failure, not an empty page
        let body = res
            .text()
            .map_err(|e| ProviderError::Unavailable(format!("ответ оборвался: {e}")))?;
        match check_response(status, &body) {
            Err(ProviderError::RateLimited { wait_secs }) => {
                let wait = retry_after.unwrap_or(wait_secs).clamp(60, 3 * 3600);
                set_cooldown_until(unix_now() + wait);
                Err(ProviderError::RateLimited { wait_secs: wait })
            }
            other => other.map(|_| body),
        }
    }

    /// Read the live tag and prefix dictionaries from the "Latest updates" page.
    fn load_dictionaries(&mut self) -> Result<(), ProviderError> {
        let url = format!("{}/sam/latest_alpha/", self.base);
        let page = self.get(&url)?;
        let (tags, prefixes) = feed::dictionaries(&page)?;
        if !tags.is_empty() {
            self.tags = tags;
            self.dictionary = "site";
        }
        self.prefixes = prefixes;
        Ok(())
    }

    pub fn list_url(page: u32) -> String {
        format!(
            "{HOST}/sam/latest_alpha/latest_data.php?cmd=list&cat=games&page={page}&sort=date&rows={PER_PAGE}"
        )
    }

    pub fn thread_url(id: &str) -> String {
        format!("{HOST}/threads/{id}/")
    }
}

impl Provider for F95Zone {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "F95Zone"
    }

    fn prepare(&mut self) -> Result<(), ProviderError> {
        match self.load_dictionaries() {
            Ok(()) => Ok(()),
            // the dictionary page may need a login: the snapshot still names most tags
            Err(ProviderError::Parse(_)) => Ok(()),
            Err(e) => Err(e),
        }
    }

    fn list(&mut self, page: u32) -> Result<ListPage, ProviderError> {
        let url = Self::list_url(page).replacen(HOST, &self.base, 1);
        let body = self.get(&url)?;
        let t = body.trim_start();
        if t.is_empty() {
            return Err(ProviderError::Unavailable("пустой ответ".into()));
        }
        if t.starts_with('<') {
            // an HTML page instead of the feed (maintenance, error page): temporary
            return Err(ProviderError::Unavailable(
                "вместо данных пришла страница сайта".into(),
            ));
        }
        feed::list(&body, &self.tags, &self.prefixes)
    }

    fn respect_cooldown(&mut self, until_unix: u64) {
        set_cooldown_until(until_unix);
    }

    fn cooldown_until(&self) -> Option<u64> {
        cooldown_until_unix()
    }

    fn slow_down(&mut self) {
        // twice as polite after a bad answer, up to one request in 8 seconds
        self.min_gap = (self.min_gap * 2).min(Duration::from_secs(8));
    }

    fn index_page(&mut self, page: u32) -> Result<super::IndexPage, ProviderError> {
        // /forums/games.2/page-N (robots.txt allows it; only addresses with "?" are disallowed)
        let url = format!("{}{GAMES_FORUM}page-{page}", self.base);
        let body = self.get(&url)?;
        thread::forum_page(&body)
    }

    fn details(&mut self, ext_id: &str) -> Result<ExtGame, ProviderError> {
        if ext_id.is_empty() || !ext_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ProviderError::Parse(format!("bad thread id {ext_id:?}")));
        }
        let url = Self::thread_url(ext_id).replacen(HOST, &self.base, 1);
        let body = self.get(&url)?;
        thread::game_page(&body, ext_id)
    }
}

/// Stop on anything that means "do not continue automatically".
pub fn check_response(status: u16, body: &str) -> Result<(), ProviderError> {
    let has = |m: &str| body.contains(m);
    if status == 429
        || has("<title>429 Too Many Requests</title>")
        || has("temporarily blocked because of a large amount of requests")
    {
        return Err(ProviderError::RateLimited {
            wait_secs: DEFAULT_COOLDOWN,
        });
    }
    if has("_cf_chl_opt")
        || has("<title>Just a moment...</title>")
        || has("<title>DDoS-Guard</title>")
        || has("<title>DDOS-GUARD</title>")
        || has("challenges.cloudflare.com")
    {
        return Err(ProviderError::Blocked(
            "проверка на робота (капча) — обходить её нельзя".into(),
        ));
    }
    if has("<title>Log in | F95zone</title>") || has("<form action=\"/login/login\"") {
        return Err(ProviderError::Blocked("страница требует входа".into()));
    }
    if status == 403 {
        return Err(ProviderError::Blocked("доступ запрещён (403)".into()));
    }
    if status >= 500 || has("<title>F95Zone :: Maintenance</title>") {
        return Err(ProviderError::Unavailable(format!(
            "ошибка сервера {status}"
        )));
    }
    if status == 404 {
        return Err(ProviderError::Unavailable("страница не найдена".into()));
    }
    if !(200..300).contains(&status) {
        return Err(ProviderError::Unavailable(format!("ответ {status}")));
    }
    Ok(())
}

// ---------- compatibility wrappers: the parsing lives in the submodules ----------

/// Snapshot of the public tag ids, used when the live dictionary cannot be read.
pub fn fallback_tags() -> HashMap<u64, String> {
    feed::fallback_tags()
}

/// Tag names and prefix names by id.
pub type Dictionaries = feed::Dictionaries;

/// `var latestUpdates = {...};` on the "Latest updates" page.
pub fn parse_dictionaries(page: &str) -> Result<Dictionaries, ProviderError> {
    feed::dictionaries(page)
}

/// One page of `latest_data.php`.
pub fn parse_list(
    body: &str,
    tags: &HashMap<u64, String>,
    prefixes: &HashMap<u64, String>,
) -> Result<ListPage, ProviderError> {
    feed::list(body, tags, prefixes)
}

/// Unescape the HTML entities the site uses.
pub fn html_unescape(s: &str) -> String {
    html::unescape(s)
}

/// Visible text of an HTML fragment, one line per block.
pub fn html_text(html: &str) -> String {
    html::text(html)
}

#[cfg(test)]
mod tests;
