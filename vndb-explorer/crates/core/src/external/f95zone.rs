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

use super::{official_site, ExtGame, ListPage, OfficialLink, Provider, ProviderError};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

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

/// Snapshot of the public tag ids, used when the live dictionary cannot be read.
const FALLBACK_TAGS: &str = include_str!("../../assets/f95_tags.json");

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

static LAST_REQUEST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

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
            tags: fallback_tags(),
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
        {
            let mut last = LAST_REQUEST.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(t) = *last {
                let since = t.elapsed();
                if since < self.min_gap {
                    std::thread::sleep(self.min_gap - since);
                }
            }
            *last = Some(Instant::now());
        }
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
        let (tags, prefixes) = parse_dictionaries(&page)?;
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
        parse_list(&body, &self.tags, &self.prefixes)
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
        parse_forum_page(&body)
    }

    fn details(&mut self, ext_id: &str) -> Result<ExtGame, ProviderError> {
        if ext_id.is_empty() || !ext_id.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ProviderError::Parse(format!("bad thread id {ext_id:?}")));
        }
        let url = Self::thread_url(ext_id).replacen(HOST, &self.base, 1);
        let body = self.get(&url)?;
        parse_thread(&body, ext_id)
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

pub fn fallback_tags() -> HashMap<u64, String> {
    serde_json::from_str::<HashMap<String, String>>(FALLBACK_TAGS)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
        .collect()
}

/// `var latestUpdates = {...};` on the "Latest updates" page: `tags` is `{id: name}`; prefixes
/// are objects `{id, name}` somewhere under a `prefixes` key.
/// Tag names and prefix names by id.
pub type Dictionaries = (HashMap<u64, String>, HashMap<u64, String>);

pub fn parse_dictionaries(page: &str) -> Result<Dictionaries, ProviderError> {
    let start = page
        .find("latestUpdates")
        .and_then(|i| page[i..].find('{').map(|j| i + j))
        .ok_or_else(|| ProviderError::Parse("нет словаря тегов на странице".into()))?;
    let v: Value = serde_json::Deserializer::from_str(&page[start..])
        .into_iter::<Value>()
        .next()
        .ok_or_else(|| ProviderError::Parse("пустой словарь".into()))?
        .map_err(|e| ProviderError::Parse(e.to_string()))?;
    let mut tags = HashMap::new();
    if let Some(obj) = v.get("tags").and_then(Value::as_object) {
        for (k, name) in obj {
            if let (Ok(id), Some(n)) = (k.parse::<u64>(), name.as_str()) {
                tags.insert(id, n.to_string());
            }
        }
    }
    let mut prefixes = HashMap::new();
    fn walk(v: &Value, inside: bool, out: &mut HashMap<u64, String>) {
        match v {
            Value::Object(o) => {
                if inside {
                    if let (Some(id), Some(name)) = (
                        o.get("id").and_then(Value::as_u64),
                        o.get("name").and_then(Value::as_str),
                    ) {
                        out.insert(id, name.to_string());
                    }
                }
                for (k, x) in o {
                    walk(x, inside || k == "prefixes", out);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, inside, out)),
            _ => {}
        }
    }
    walk(&v, false, &mut prefixes);
    Ok((tags, prefixes))
}

fn status_of(name: &str) -> Option<&'static str> {
    match name.to_lowercase().replace(' ', "").as_str() {
        "completed" => Some("completed"),
        "onhold" => Some("onhold"),
        "abandoned" => Some("abandoned"),
        _ => None,
    }
}

fn engine_of(name: &str) -> Option<&'static str> {
    ENGINES
        .iter()
        .find(|e| e.eq_ignore_ascii_case(name.trim()))
        .copied()
}

/// Days since 1970-01-01 → YYYY-MM-DD (proleptic Gregorian).
fn date_of(secs: i64) -> String {
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

fn num(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// One page of `latest_data.php`.
pub fn parse_list(
    body: &str,
    tags: &HashMap<u64, String>,
    prefixes: &HashMap<u64, String>,
) -> Result<ListPage, ProviderError> {
    let v: Value = serde_json::from_str(body)
        .map_err(|e| ProviderError::Parse(format!("ответ не JSON: {e}")))?;
    if v.get("status").and_then(Value::as_str) != Some("ok") {
        let msg = v.get("msg").map(|m| m.to_string()).unwrap_or_default();
        if msg.contains("temporarily blocked") {
            set_cooldown_until(unix_now() + DEFAULT_COOLDOWN);
            return Err(ProviderError::RateLimited {
                wait_secs: DEFAULT_COOLDOWN,
            });
        }
        return Err(ProviderError::Parse(format!("ошибка ленты: {msg}")));
    }
    let msg = &v["msg"];
    let data = msg
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| ProviderError::Parse("нет списка игр".into()))?;
    let mut games = Vec::new();
    let mut bad_items = 0;
    for it in data {
        let Some(id) = it.get("thread_id").and_then(num) else {
            bad_items += 1;
            continue;
        };
        let str_of = |k: &str| {
            it.get(k)
                .and_then(Value::as_str)
                .map(|s| html_unescape(s).trim().to_string())
                .unwrap_or_default()
        };
        let ids = |k: &str| -> Vec<u64> {
            it.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(num).collect())
                .unwrap_or_default()
        };
        let mut status = String::new();
        let mut engine = String::new();
        for p in ids("prefixes") {
            if let Some(name) = prefixes.get(&p) {
                if let Some(s) = status_of(name) {
                    status = s.into();
                } else if let Some(e) = engine_of(name) {
                    engine = e.into();
                }
            }
        }
        if status.is_empty() && !prefixes.is_empty() {
            status = "ongoing".into();
        }
        let mut version = str_of("version");
        if version.eq_ignore_ascii_case("unknown") {
            version.clear();
        }
        let updated = it.get("ts").and_then(num).map(|t| date_of(t as i64));
        games.push(ExtGame {
            source: ID.into(),
            ext_id: id.to_string(),
            title: str_of("title"),
            developer: str_of("creator"),
            version,
            status,
            engine,
            tags: ids("tags")
                .iter()
                .filter_map(|t| tags.get(t).cloned())
                .collect(),
            cover: it
                .get("cover")
                .and_then(Value::as_str)
                .filter(|c| c.starts_with("https://"))
                .map(String::from),
            rating: it.get("rating").and_then(Value::as_f64).map(|r| r as f32),
            likes: it.get("likes").and_then(num).unwrap_or(0) as u32,
            views: it.get("views").and_then(num).unwrap_or(0) as u32,
            updated,
            page_url: F95Zone::thread_url(&id.to_string()),
            description: None,
            links: Vec::new(),
        });
    }
    let total_pages = msg
        .get("pagination")
        .and_then(|p| p.get("total"))
        .and_then(num)
        .map(|t| t as u32);
    Ok(ListPage {
        games,
        total_pages,
        bad_items,
    })
}

// ---------- live check ----------

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CheckStep {
    pub name: String,
    pub ok: bool,
    pub details: Vec<String>,
}

/// A short, polite test against the real site: about 4 + `games` requests, 3 s apart.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct LiveCheck {
    pub steps: Vec<CheckStep>,
    /// Releases in the feed, counted from its first and last page
    pub feed_releases: Option<usize>,
    /// Threads in the Games forum list, counted the same way (pinned ones left out)
    pub forum_threads: Option<usize>,
    pub ok: bool,
    pub stopped: Option<String>,
    pub at: String,
}

/// Exact size of a paged list from its first and last page.
fn size(per_page: usize, last_page: u32, on_last: usize) -> usize {
    per_page * (last_page.saturating_sub(1) as usize) + on_last
}

pub fn live_check(
    conn: &rusqlite::Connection,
    games: usize,
    log: &dyn Fn(&str),
) -> anyhow::Result<LiveCheck> {
    let mut p = F95Zone::new();
    super::load_cooldown(&mut p, conn)?;
    if let Some(left) = cooldown_left() {
        log(&format!(
            "Сайт просил паузу: проверка подождёт ещё {} мин",
            left.div_ceil(60)
        ));
        std::thread::sleep(Duration::from_secs(left));
    }
    let mut out = LiveCheck {
        at: super::store::stamp(),
        ..Default::default()
    };
    let step = |out: &mut LiveCheck, name: &str, ok: bool, details: Vec<String>| {
        log(&format!(
            "{} {name}: {}",
            if ok { "✓" } else { "✗" },
            details.join("; ")
        ));
        out.steps.push(CheckStep {
            name: name.into(),
            ok,
            details,
        });
    };
    let stop = |out: &mut LiveCheck, e: &ProviderError| -> bool {
        if let ProviderError::Blocked(_) | ProviderError::RateLimited { .. } = e {
            out.stopped = Some(e.to_string());
            return true;
        }
        false
    };
    // 1. dictionaries
    match p.load_dictionaries() {
        Ok(()) => step(
            &mut out,
            "Словарь тегов",
            true,
            vec![
                format!(
                    "источник: {}",
                    if p.dictionary == "site" {
                        "сайт"
                    } else {
                        "встроенный снимок"
                    }
                ),
                format!("тегов: {}, префиксов: {}", p.tags.len(), p.prefixes.len()),
            ],
        ),
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Словарь тегов",
                false,
                vec![e.to_string(), "используется встроенный снимок тегов".into()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    // 2-3. the feed: first and last page give its exact size
    let mut sample: Vec<ExtGame> = Vec::new();
    match p.list(1) {
        Ok(first) => {
            let total = first.total_pages.unwrap_or(1);
            let mut d = vec![
                format!("игр на странице: {}, страниц: {}", first.games.len(), total),
                format!(
                    "с тегами: {}, со статусом: {}, нечитаемых записей: {}",
                    first.games.iter().filter(|g| !g.tags.is_empty()).count(),
                    first.games.iter().filter(|g| !g.status.is_empty()).count(),
                    first.bad_items
                ),
            ];
            d.extend(
                first
                    .games
                    .iter()
                    .take(3)
                    .map(|g| format!("«{}» — {}, {} тегов", g.title, g.developer, g.tags.len())),
            );
            let ok = !first.games.is_empty() && first.games.iter().any(|g| !g.tags.is_empty());
            step(&mut out, "Лента: первая страница", ok, d);
            for g in &first.games {
                super::store::upsert_listed(conn, g, "feed")?;
            }
            sample = first.games.clone();
            if total > 1 {
                match p.list(total) {
                    Ok(last) => {
                        // the page size the site really uses, not the one asked for
                        let n = size(first.games.len(), total, last.games.len());
                        out.feed_releases = Some(n);
                        step(
                            &mut out,
                            "Лента: последняя страница",
                            true,
                            vec![format!(
                                "на последней странице {}, всего в ленте {n}",
                                last.games.len()
                            )],
                        );
                    }
                    Err(e) => {
                        let fatal = stop(&mut out, &e);
                        step(
                            &mut out,
                            "Лента: последняя страница",
                            false,
                            vec![e.to_string()],
                        );
                        if fatal {
                            return finish(conn, out);
                        }
                    }
                }
            } else {
                out.feed_releases = Some(first.games.len());
            }
        }
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Лента: первая страница",
                false,
                vec![e.to_string()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    // 4-5. the Games forum list: the site's own index of every game thread
    match p.index_page(1) {
        Ok(first) => {
            let per = first.games.len();
            let last_page = first.last_page.unwrap_or(1);
            let mut d = vec![format!(
                "тем на странице: {per} (+ закреплённых служебных: {}), страниц: {last_page}",
                first.service
            )];
            d.extend(first.games.iter().take(3).map(|g| {
                format!(
                    "«{}» — {}",
                    g.title,
                    if g.developer.is_empty() {
                        "разработчик не указан"
                    } else {
                        &g.developer
                    }
                )
            }));
            step(&mut out, "Форум Games: первая страница", per > 0, d);
            if last_page > 1 && per > 0 {
                match p.index_page(last_page) {
                    Ok(last) => {
                        let n = size(per, last_page, last.games.len());
                        out.forum_threads = Some(n);
                        step(
                            &mut out,
                            "Форум Games: последняя страница",
                            true,
                            vec![format!(
                                "на последней странице {}, всего тем ≈ {n}",
                                last.games.len()
                            )],
                        );
                    }
                    Err(e) => {
                        let fatal = stop(&mut out, &e);
                        step(
                            &mut out,
                            "Форум Games: последняя страница",
                            false,
                            vec![e.to_string()],
                        );
                        if fatal {
                            return finish(conn, out);
                        }
                    }
                }
            } else {
                out.forum_threads = Some(per);
            }
        }
        Err(e) => {
            let fatal = stop(&mut out, &e);
            step(
                &mut out,
                "Форум Games: первая страница",
                false,
                vec![e.to_string()],
            );
            if fatal {
                return finish(conn, out);
            }
        }
    }
    if let (Some(f), Some(t)) = (out.feed_releases, out.forum_threads) {
        let diff = t as i64 - f as i64;
        step(
            &mut out,
            "Сравнение ленты и форума",
            diff <= 0,
            vec![
                format!("в ленте {f}, в форуме {t}"),
                if diff > 0 {
                    format!("в форуме на {diff} тем больше: лента неполная, нужна сверка с форумом")
                } else {
                    "в ленте не меньше тем, чем в форуме".into()
                },
            ],
        );
    }
    // 6. a few game pages
    for g in sample.iter().take(games) {
        match p.details(&g.ext_id) {
            Ok(d) => {
                let change = super::store::save_details(conn, &d)?;
                let ok = !d.title.is_empty() && !d.tags.is_empty() && d.description.is_some();
                step(
                    &mut out,
                    &format!("Страница игры {}", g.ext_id),
                    ok,
                    vec![
                        format!(
                            "«{}», разработчик: {}",
                            d.title,
                            if d.developer.is_empty() {
                                "—"
                            } else {
                                &d.developer
                            }
                        ),
                        format!(
                            "тегов: {}, описание: {} знаков, обложка: {}",
                            d.tags.len(),
                            d.description.as_ref().map_or(0, |x| x.chars().count()),
                            if d.cover.is_some() {
                                "есть"
                            } else {
                                "нет"
                            }
                        ),
                        format!(
                            "официальные ссылки: {}",
                            if d.links.is_empty() {
                                "нет".to_string()
                            } else {
                                d.links
                                    .iter()
                                    .map(|l| l.site.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            }
                        ),
                        if let super::store::Change::Hidden(r) = change {
                            format!("скрыта правилами ({r})")
                        } else {
                            "сохранена".into()
                        },
                    ],
                );
            }
            Err(e) => {
                let fatal = stop(&mut out, &e);
                step(
                    &mut out,
                    &format!("Страница игры {}", g.ext_id),
                    false,
                    vec![e.to_string()],
                );
                if fatal {
                    break;
                }
            }
        }
    }
    finish(conn, out)
}

fn finish(conn: &rusqlite::Connection, mut out: LiveCheck) -> anyhow::Result<LiveCheck> {
    if let Some(t) = cooldown_until_unix() {
        super::store::set_meta(conn, ID, "cooldown_until", &t.to_string())?;
    }
    out.ok = out.stopped.is_none() && !out.steps.is_empty() && out.steps.iter().all(|s| s.ok);
    super::store::set_meta(conn, ID, "last_check", &serde_json::to_string(&out)?)?;
    Ok(out)
}

// ---------- the game page (HTML) ----------

pub fn html_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest[..rest.len().min(12)].find(';');
        let Some(end) = end else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ if ent.starts_with("#x") || ent.starts_with("#X") => {
                u32::from_str_radix(&ent[2..], 16)
                    .ok()
                    .and_then(char::from_u32)
            }
            _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
            _ => None,
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Visible text of an HTML fragment, one line per block.
pub fn html_text(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(i) = rest.find('<') {
        out.push_str(&rest[..i]);
        let Some(j) = rest[i..].find('>') else {
            rest = "";
            break;
        };
        let tag = rest[i + 1..i + j].trim_start_matches('/').to_lowercase();
        let name = tag.split([' ', '/']).next().unwrap_or("");
        if matches!(
            name,
            "br" | "div" | "p" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "blockquote"
        ) {
            out.push('\n');
        }
        // drop script / style contents
        if !rest[i + 1..].starts_with('/') && (name == "script" || name == "style") {
            let close = format!("</{name}");
            rest = match rest[i..].to_lowercase().find(&close) {
                Some(k) => &rest[i + k..],
                None => "",
            };
            continue;
        }
        rest = &rest[i + j + 1..];
    }
    out.push_str(rest);
    let text = html_unescape(&out);
    let mut lines: Vec<String> = Vec::new();
    for l in text.lines() {
        let l = l.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() && lines.last().is_none_or(|x| x.is_empty()) {
            continue;
        }
        lines.push(l);
    }
    lines.join("\n").trim().to_string()
}

fn between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = s.find(start)? + start.len();
    let j = s[i..].find(end)?;
    Some(&s[i..i + j])
}

fn attr_values<'a>(html: &'a str, attr: &str) -> Vec<(usize, &'a str)> {
    let pat = format!("{attr}=\"");
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(i) = html[from..].find(&pat) {
        let s = from + i + pat.len();
        let Some(e) = html[s..].find('"') else { break };
        out.push((from + i, &html[s..s + e]));
        from = s + e;
    }
    out
}

/// `Name: value` on its own line in the first post.
fn field(text: &str, names: &[&str]) -> Option<String> {
    for line in text.lines() {
        let l = line.trim();
        for n in names {
            if l.len() > n.len()
                && l[..n.len()].eq_ignore_ascii_case(n)
                && l[n.len()..].trim_start().starts_with(':')
            {
                let v = l[n.len()..].trim_start()[1..].trim();
                if !v.is_empty() {
                    return Some(v.to_string());
                }
            }
        }
    }
    None
}

/// Free text after "Overview:" up to the next `Header:` line.
fn overview(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let i = ["overview", "story", "synopsis", "description"]
        .iter()
        .filter_map(|k| {
            let i = lower.find(&format!("{k}:"))?;
            Some(i + k.len() + 1)
        })
        .min()?;
    let mut out: Vec<&str> = Vec::new();
    for line in text[i..].lines() {
        let l = line.trim();
        let is_header = l.len() < 40 && l.ends_with(':') | l.contains(": ") && {
            let head = l.split(':').next().unwrap_or("");
            !head.is_empty()
                && head.len() < 30
                && head.chars().next().is_some_and(|c| c.is_uppercase())
                && head
                    .chars()
                    .all(|c| c.is_alphanumeric() || c == ' ' || c == '-' || c == '/')
        };
        if !out.is_empty() && (is_header || l.eq_ignore_ascii_case("download")) {
            break;
        }
        if !l.is_empty() || !out.is_empty() {
            out.push(l);
        }
    }
    let s = out.join("\n").trim().to_string();
    if s.is_empty() {
        return None;
    }
    // the whole public overview, read only when the user opens the game (like a browser);
    // a safety limit against pages that swallow the rest of the post
    Some(excerpt(&s, DESCRIPTION_MAX))
}

fn excerpt(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    let end = cut
        .rfind(['.', '!', '?'])
        .filter(|&i| i > max / 2)
        .map(|i| i + 1);
    format!("{}…", cut[..end.unwrap_or(cut.len())].trim_end())
}

/// Where the download section starts in the first post (everything after it is ignored).
fn download_section(body_lower: &str) -> usize {
    [
        ">download<",
        ">download:",
        ">downloads<",
        "<b>download",
        "download</b>",
        "download</span>",
    ]
    .iter()
    .filter_map(|m| body_lower.find(m))
    .min()
    .unwrap_or(body_lower.len())
}

/// Split "Name [v0.5] [Developer]" into name and bracket parts.
fn split_title(t: &str) -> (String, Vec<String>) {
    let mut brackets: Vec<String> = Vec::new();
    let mut name = t.trim().to_string();
    while name.ends_with(']') {
        let Some(i) = name.rfind('[') else { break };
        brackets.insert(0, name[i + 1..name.len() - 1].trim().to_string());
        name = name[..i].trim().to_string();
    }
    (name, brackets)
}

/// Prefix labels inside an HTML fragment: `<span class="label ...">Completed</span>`.
fn labels(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("<span class=\"label") {
        let Some(gt) = rest[i..].find('>') else { break };
        let s = i + gt + 1;
        let Some(e) = rest[s..].find("</span>") else {
            break;
        };
        out.push(html_text(&rest[s..s + e]));
        rest = &rest[s + e..];
    }
    out
}

/// One page of the Games forum thread list (XenForo): thread id, title with prefixes.
/// Pinned threads (rules, announcements) are counted as service threads and left out.
pub fn parse_forum_page(html: &str) -> Result<super::IndexPage, ProviderError> {
    check_response(200, html)?;
    let mut out = super::IndexPage::default();
    // page numbers of the pager: the largest one is the last page
    let marker = format!("{GAMES_FORUM}page-");
    let mut from = 0;
    while let Some(i) = html[from..].find(&marker) {
        let s = from + i + marker.len();
        let n: String = html[s..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Ok(n) = n.parse::<u32>() {
            out.last_page = Some(out.last_page.map_or(n, |m| m.max(n)));
        }
        from = s;
    }
    let sticky_end = html
        .find("structItemContainer-group--sticky")
        .and_then(|i| {
            html[i + 10..]
                .find("structItemContainer-group")
                .map(|j| i + 10 + j)
        });
    const ITEM: &str = "js-threadListItem-";
    let starts: Vec<usize> = html.match_indices(ITEM).map(|(i, _)| i).collect();
    if starts.is_empty() && !html.contains("structItemContainer") {
        return Err(ProviderError::Parse("нет списка тем".into()));
    }
    for (k, &i) in starts.iter().enumerate() {
        let end = starts.get(k + 1).copied().unwrap_or(html.len());
        let item = &html[i..end];
        let id: String = item[ITEM.len()..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if id.is_empty() {
            continue;
        }
        let sticky = sticky_end.is_some_and(|e| i < e) || item.contains("structItem--sticky");
        if sticky {
            out.service += 1;
            continue;
        }
        // the thread link inside the title block
        let title_html = item
            .find("structItem-title")
            .map(|t| &item[t..])
            .and_then(|t| t.find("</div>").map(|e| &t[..e]))
            .unwrap_or(item);
        let link = title_html
            .match_indices("href=\"/threads/")
            .last()
            .map(|(j, _)| j)
            .and_then(|j| {
                let a = &title_html[j..];
                let g = a.find('>')?;
                let e = a[g..].find("</a>")?;
                Some(html_text(&a[g + 1..g + e]))
            })
            .unwrap_or_default();
        let (name, brackets) = split_title(&link);
        let mut status = "ongoing".to_string();
        let mut engine = String::new();
        for l in labels(title_html) {
            if let Some(s) = status_of(&l) {
                status = s.into();
            } else if let Some(e) = engine_of(&l) {
                engine = e.into();
            }
        }
        out.games.push(ExtGame {
            source: ID.into(),
            ext_id: id.clone(),
            title: name,
            developer: if brackets.len() >= 2 {
                brackets.last().cloned().unwrap_or_default()
            } else {
                String::new()
            },
            version: brackets
                .first()
                .cloned()
                .filter(|_| brackets.len() >= 2)
                .unwrap_or_default(),
            status,
            engine,
            page_url: F95Zone::thread_url(&id),
            ..Default::default()
        });
    }
    Ok(out)
}

/// The public game page: title, prefixes, tags, first post (description, developer links).
pub fn parse_thread(html: &str, id: &str) -> Result<ExtGame, ProviderError> {
    check_response(200, html)?;
    if html.contains("data-template=\"error\"") {
        return Err(ProviderError::Unavailable("тема удалена или скрыта".into()));
    }
    let h1 = between(html, "<h1 class=\"p-title-value\"", "</h1>")
        .ok_or_else(|| ProviderError::Parse("нет заголовка темы".into()))?;
    let h1 = h1.split_once('>').map(|x| x.1).unwrap_or(h1);
    // prefix labels: <span class="label ...">Completed</span> (also inside <a class="labelLink">)
    let mut labels = Vec::new();
    let mut rest = h1;
    while let Some(i) = rest.find("<span class=\"label") {
        let Some(gt) = rest[i..].find('>') else { break };
        let s = i + gt + 1;
        let Some(e) = rest[s..].find("</span>") else {
            break;
        };
        labels.push(html_text(&rest[s..s + e]));
        rest = &rest[s + e..];
    }
    let mut title_text = html_text(h1);
    for l in &labels {
        if let Some(t) = title_text.strip_prefix(l.as_str()) {
            title_text = t.trim_start().to_string();
        }
    }
    let title_text = title_text.replace('\n', " ");
    // "Name [v0.5] [Developer]"
    let (name, brackets) = split_title(&title_text);
    let mut status = "ongoing".to_string();
    let mut engine = String::new();
    for l in &labels {
        if let Some(s) = status_of(l) {
            status = s.into();
        } else if let Some(e) = engine_of(l) {
            engine = e.into();
        }
    }
    // tags
    let mut tags = Vec::new();
    if let Some(i) = html.find("js-tagList") {
        let end = html[i..]
            .find("</dd>")
            .or_else(|| html[i..].find("</div>"))
            .unwrap_or(0);
        for (_, href) in attr_values(&html[i..i + end], "href") {
            if let Some(slug) = href.strip_prefix("/tags/") {
                let t = slug.trim_matches('/').replace('-', " ");
                if !t.is_empty() && !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
    }
    // first post
    let post_start = html
        .find("<div class=\"bbWrapper\">")
        .ok_or_else(|| ProviderError::Parse("нет первого сообщения".into()))?;
    let post_end = html[post_start..]
        .find("</article>")
        .map(|e| post_start + e)
        .unwrap_or(html.len());
    let post = &html[post_start..post_end];
    let cut = download_section(&post.to_lowercase());
    let head = &post[..cut];
    let text = html_text(head);
    let mut links: Vec<OfficialLink> = Vec::new();
    let mut seen = HashSet::new();
    for (pos, href) in attr_values(head, "href") {
        let url = html_unescape(href);
        let Some(site) = official_site(&url) else {
            continue;
        };
        if !seen.insert(super::matching::norm_url(&url)) {
            continue;
        }
        let label = head[pos..]
            .find('>')
            .and_then(|g| head[pos + g + 1..].find("</a>").map(|e| (pos + g + 1, e)))
            .map(|(s, e)| html_text(&head[s..s + e]))
            .filter(|l| !l.is_empty() && l.len() < 60)
            .unwrap_or_else(|| site.to_string());
        links.push(OfficialLink {
            site: site.into(),
            label,
            url,
        });
    }
    let developer = field(&text, &["Developer", "Developer/Publisher", "Artist"])
        .map(|d| d.split(" - ").next().unwrap_or("").trim().to_string())
        .filter(|d| !d.is_empty())
        .or_else(|| brackets.last().cloned())
        .unwrap_or_default();
    let version = field(&text, &["Version"])
        .or_else(|| brackets.first().cloned().filter(|b| b != &developer))
        .unwrap_or_default();
    let updated = field(&text, &["Thread Updated", "Updated", "Release Date"]).and_then(|d| {
        let d = d.replace('/', "-");
        let ok = d.len() >= 10
            && d[..10].chars().enumerate().all(|(i, c)| {
                if i == 4 || i == 7 {
                    c == '-'
                } else {
                    c.is_ascii_digit()
                }
            });
        ok.then(|| d[..10].to_string())
    });
    let cover = attr_values(head, "data-src")
        .into_iter()
        .chain(attr_values(head, "src"))
        .map(|(_, u)| html_unescape(u))
        .find(|u| {
            u.starts_with("https://attachments.f95zone.to/")
                || u.starts_with("https://preview.f95zone.to/")
        });
    Ok(ExtGame {
        source: ID.into(),
        ext_id: id.to_string(),
        title: name,
        developer,
        version,
        status,
        engine,
        tags,
        cover,
        rating: None,
        likes: 0,
        views: 0,
        updated,
        page_url: F95Zone::thread_url(id),
        description: overview(&text),
        links,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"status":"ok","msg":{"data":[
      {"thread_id":319968,"title":"La Previa","creator":"rioplatagames","version":"v1.3","views":18,"likes":0,
       "prefixes":[13,7,18],"tags":[130,173,237],"rating":0,"cover":"https:\/\/preview.f95zone.to\/2026\/11\/x.png",
       "screens":[],"date":"2 hrs","watched":false,"ignored":false,"new":true,"ts":1794222906},
      {"thread_id":"1000","title":"Old &amp; Gold","creator":"Dev","version":"Unknown","views":"5","likes":2,
       "prefixes":[],"tags":[75],"rating":4.5,"cover":"","screens":[],"ts":0}
    ],"pagination":{"page":1,"total":210},"count":18900}}"#;

    #[test]
    fn list_feed() {
        let mut tags = fallback_tags();
        tags.insert(130, "3dcg".into());
        let prefixes: HashMap<u64, String> = [
            (7, "Ren'Py".to_string()),
            (18, "Completed".to_string()),
            (13, "VN".to_string()),
        ]
        .into_iter()
        .collect();
        let p = parse_list(LIST, &tags, &prefixes).unwrap();
        assert_eq!(p.total_pages, Some(210));
        let g = &p.games[0];
        assert_eq!(g.ext_id, "319968");
        assert_eq!(g.title, "La Previa");
        assert_eq!(g.developer, "rioplatagames");
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("completed", "Ren'Py")
        );
        assert!(g.tags.contains(&"3dcg".to_string()));
        assert_eq!(g.updated.as_deref(), Some("2026-11-09"));
        assert_eq!(g.page_url, "https://f95zone.to/threads/319968/");
        let o = &p.games[1];
        assert_eq!(o.title, "Old & Gold");
        assert_eq!(o.version, "");
        assert_eq!(o.views, 5);
        assert_eq!(o.cover, None);
        assert_eq!(o.updated.as_deref(), Some("1970-01-01"));
        // the snapshot names tag 75, so the content policy can see it
        assert!(super::super::is_forbidden(&o.tags));
    }

    #[test]
    fn errors_stop_the_import() {
        let blocked = r#"{"status":"error","msg":"You have been temporarily blocked because of a large amount of requests, please try again later"}"#;
        assert!(matches!(
            parse_list(blocked, &HashMap::new(), &HashMap::new()),
            Err(ProviderError::RateLimited { .. })
        ));
        assert!(matches!(
            check_response(200, "<html><title>Just a moment...</title>"),
            Err(ProviderError::Blocked(_))
        ));
        assert!(matches!(
            check_response(429, ""),
            Err(ProviderError::RateLimited { .. })
        ));
        assert!(matches!(
            check_response(403, ""),
            Err(ProviderError::Blocked(_))
        ));
        assert!(matches!(
            check_response(502, ""),
            Err(ProviderError::Unavailable(_))
        ));
        assert!(check_response(200, "{}").is_ok());
    }

    #[test]
    fn dictionaries_from_the_page() {
        let page = r#"<script>var latestUpdates = {"tags":{"1":"2d game","130":"3dcg"},
          "options":{"prefixes":{"games":[{"id":1,"name":"Engine","prefixes":[{"id":7,"name":"Ren'Py"},{"id":18,"name":"Completed"}]}]}}};
          var other = 1;</script>"#;
        let (t, p) = parse_dictionaries(page).unwrap();
        assert_eq!(t.get(&130).map(String::as_str), Some("3dcg"));
        assert_eq!(p.get(&7).map(String::as_str), Some("Ren'Py"));
        assert_eq!(p.get(&18).map(String::as_str), Some("Completed"));
        assert!(parse_dictionaries("<html>login</html>").is_err());
    }

    const THREAD: &str = r#"<html><head><title>Summer Nights [v0.5] [Acme] | F95zone</title></head><body>
      <h1 class="p-title-value"><a href="/forums/games.2/?prefix_id=7" class="labelLink"><span class="label label--gray" dir="auto">Ren'Py</span></a><span class="label-append">&nbsp;</span><span class="label label--blue" dir="auto">Completed</span><span class="label-append">&nbsp;</span>Summer Nights [v0.5] [Acme]</h1>
      <dd><span class="js-tagList"><a href="/tags/3dcg/" class="tagItem">3dcg</a><a href="/tags/male-protagonist/" class="tagItem">male protagonist</a></span></dd>
      <article class="message"><div class="bbWrapper"><div style="text-align: center"><img src="https://attachments.f95zone.to/2026/11/cover.png" data-src="https://attachments.f95zone.to/2026/11/cover.png" /></div>
      <b>Overview:</b><br />A quiet town &amp; one long summer.<br />Make choices that matter.<br /><br />
      <b>Thread Updated</b>: 2026-10-30<br /><b>Release Date</b>: 2026-10-29<br />
      <b>Developer</b>: Acme - <a href="https://www.patreon.com/acme" target="_blank">Patreon</a> - <a href="https://acme.itch.io/summer-nights">Itch.io</a> - <a href="https://f95zone.to/masked/xyz">Site</a><br />
      <b>Censored</b>: No<br /><b>Version</b>: 0.5<br />
      <b>DOWNLOAD</b><br /><a href="https://mega.nz/file/abc">MEGA</a> - <a href="https://www.patreon.com/posts/build-123">Patreon build</a>
      </div></article></body></html>"#;

    #[test]
    fn game_page() {
        let g = parse_thread(THREAD, "4242").unwrap();
        assert_eq!(g.title, "Summer Nights");
        assert_eq!(g.developer, "Acme");
        assert_eq!(g.version, "0.5");
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("completed", "Ren'Py")
        );
        assert_eq!(g.tags, vec!["3dcg", "male protagonist"]);
        assert_eq!(g.updated.as_deref(), Some("2026-10-30"));
        assert_eq!(
            g.description.as_deref(),
            Some("A quiet town & one long summer.\nMake choices that matter.")
        );
        let urls: Vec<&str> = g.links.iter().map(|l| l.url.as_str()).collect();
        // only official pages above the download section; no mirrors, no forum redirects
        assert_eq!(
            urls,
            vec![
                "https://www.patreon.com/acme",
                "https://acme.itch.io/summer-nights"
            ]
        );
        assert_eq!(g.links[1].label, "Itch.io");
        assert_eq!(
            g.cover.as_deref(),
            Some("https://attachments.f95zone.to/2026/11/cover.png")
        );
        assert!(matches!(
            parse_thread("<html><title>Log in | F95zone</title></html>", "1"),
            Err(ProviderError::Blocked(_))
        ));
    }

    const FORUM: &str = r#"<html><body>
      <div class="structItemContainer-group structItemContainer-group--sticky">
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-5"><div class="structItem-title">
          <a href="/threads/f95zone-games-rules.5/">F95Zone Games Rules</a></div></div>
      </div>
      <div class="structItemContainer-group js-threadList">
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-319968"><div class="structItem-title">
          <a href="/forums/games.2/?prefix_id=7" class="labelLink" rel="nofollow"><span class="label label--gray" dir="auto">Ren'Py</span></a>
          <a href="/threads/la-previa.319968/" data-tp-primary="on">La Previa [v1.3] [rioplatagames]</a></div></div>
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-1000"><div class="structItem-title">
          <a href="/forums/games.2/?prefix_id=18" class="labelLink"><span class="label label--blue">Completed</span></a>
          <a href="/forums/games.2/?prefix_id=3" class="labelLink"><span class="label label--gray">Unity</span></a>
          <a href="/threads/old-gold.1000/">Old &amp; Gold [Final] [Dev Team]</a></div></div>
        <div class="structItem structItem--thread js-inlineModContainer js-threadListItem-1001"><div class="structItem-title">
          <a href="/threads/no-brackets.1001/">No Brackets</a></div></div>
      </div>
      <ul class="pageNav-main"><li><a href="/forums/games.2/page-2">2</a></li><li><a href="/forums/games.2/page-987">987</a></li></ul>
    </body></html>"#;

    #[test]
    fn forum_list() {
        let p = parse_forum_page(FORUM).unwrap();
        assert_eq!(p.last_page, Some(987));
        assert_eq!(p.service, 1);
        let ids: Vec<&str> = p.games.iter().map(|g| g.ext_id.as_str()).collect();
        assert_eq!(ids, vec!["319968", "1000", "1001"]);
        let g = &p.games[0];
        assert_eq!(
            (g.title.as_str(), g.version.as_str(), g.developer.as_str()),
            ("La Previa", "v1.3", "rioplatagames")
        );
        assert_eq!(
            (g.status.as_str(), g.engine.as_str()),
            ("ongoing", "Ren'Py")
        );
        let o = &p.games[1];
        assert_eq!(
            (
                o.title.as_str(),
                o.developer.as_str(),
                o.status.as_str(),
                o.engine.as_str()
            ),
            ("Old & Gold", "Dev Team", "completed", "Unity")
        );
        assert_eq!(
            (p.games[2].title.as_str(), p.games[2].developer.as_str()),
            ("No Brackets", "")
        );
        assert!(matches!(
            parse_forum_page("<html><title>Just a moment...</title></html>"),
            Err(ProviderError::Blocked(_))
        ));
        assert!(parse_forum_page("<html>nothing</html>").is_err());
    }

    #[test]
    fn whole_description_is_kept() {
        let long: String = (0..400)
            .map(|i| format!("Sentence number {i} of the story. "))
            .collect();
        let page = THREAD.replace(
            "A quiet town &amp; one long summer.<br />Make choices that matter.",
            &long,
        );
        let g = parse_thread(&page, "1").unwrap();
        let d = g.description.unwrap();
        assert!(d.chars().count() > 10_000, "{}", d.len());
        assert!(d.ends_with("story."));
    }

    #[test]
    fn dates_and_entities() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(
            html_unescape("a &amp; b &#39;c&#x27; &bogus d"),
            "a & b 'c' &bogus d"
        );
        assert_eq!(excerpt("One. Two. Three.", 10), "One. Two.…");
    }
}
