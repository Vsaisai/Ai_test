//! REST API. Every response is JSON; the same operations can later be exposed to a mobile
//! shell (e.g. Tauri commands) by calling `vndb_core` directly.

use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use rusqlite::Connection;
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use tower_http::compression::predicate::Predicate as _;
use vndb_core::search::{Card, SearchRequest, Sort};
use vndb_core::shelves::{self, Shelves};
use vndb_core::sources::Sources;
use vndb_core::Catalog;

#[path = "external_api.rs"]
mod external;
pub use external::start_refresh as start_external_refresh;

#[derive(RustEmbed)]
#[folder = "../../web/"]
struct Assets;

/// Background job (full import, API sync or source indexing); one at a time.
#[derive(Default, Clone, Serialize)]
pub struct JobStatus {
    pub kind: String,
    pub running: bool,
    pub log: Vec<String>,
    pub error: Option<String>,
    pub finished: bool,
    /// Live progress (see [`progress_view`]).
    pub progress: Option<Value>,
    #[serde(skip)]
    track: Track,
}

/// Speed / time bookkeeping for the progress view.
#[derive(Clone)]
struct Track {
    started: std::time::Instant,
    stage: &'static str,
    last_t: std::time::Instant,
    last_done: u64,
    speed: f64,
}

impl Default for Track {
    fn default() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            stage: "",
            last_t: now,
            last_done: 0,
            speed: 0.0,
        }
    }
}

fn fmt_mb(b: f64) -> String {
    let mb = b / 1_048_576.0;
    if mb >= 10.0 {
        format!("{mb:.0}")
    } else {
        format!("{mb:.1}").replace('.', ",")
    }
}

fn fmt_dur(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// Turn a raw [`Progress`] into what the UI and the Android notification show:
/// stage and overall percentages, smoothed speed, time left in the stage, elapsed time,
/// and a ready-made Russian summary line.
fn progress_view(kind: &str, p: &vndb_core::progress::Progress, tr: &mut Track) -> Value {
    use vndb_core::progress::{stage_label, stage_weight, stages_of};
    let now = std::time::Instant::now();
    if tr.stage != p.stage {
        tr.stage = p.stage;
        tr.last_t = now;
        tr.last_done = p.done;
        tr.speed = 0.0;
    } else {
        let dt = now.duration_since(tr.last_t).as_secs_f64();
        if dt >= 0.7 && p.done >= tr.last_done {
            let inst = (p.done - tr.last_done) as f64 / dt;
            tr.speed = if tr.speed == 0.0 {
                inst
            } else {
                tr.speed * 0.7 + inst * 0.3
            };
            tr.last_t = now;
            tr.last_done = p.done;
        }
    }
    let frac = p
        .total
        .filter(|t| *t > 0)
        .map(|t| (p.done as f64 / t as f64).min(1.0));
    let stages = stages_of(kind);
    let all: f64 = stages.iter().map(|s| stage_weight(s)).sum();
    let pos = stages.iter().position(|s| *s == p.stage);
    let before: f64 = pos
        .map(|i| stages[..i].iter().map(|s| stage_weight(s)).sum())
        .unwrap_or(0.0);
    let overall = ((before + stage_weight(p.stage) * frac.unwrap_or(0.0)) / all * 100.0).min(100.0);
    let eta = match (p.total, tr.speed > 0.0) {
        (Some(t), true) if t > p.done => Some((t - p.done) as f64 / tr.speed),
        _ => None,
    };
    let mut parts = Vec::new();
    match (p.unit, p.total) {
        ("bytes", Some(t)) => parts.push(format!(
            "{} из {} МБ",
            fmt_mb(p.done as f64),
            fmt_mb(t as f64)
        )),
        ("bytes", None) if p.done > 0 => parts.push(format!("{} МБ", fmt_mb(p.done as f64))),
        (_, Some(t)) => parts.push(format!("{} из {}", p.done, t)),
        _ => {}
    }
    if tr.speed > 0.0 {
        parts.push(if p.unit == "bytes" {
            format!("{} МБ/с", fmt_mb(tr.speed))
        } else {
            format!("{:.0} в сек.", tr.speed)
        });
    }
    if let Some(e) = eta {
        parts.push(format!("осталось ~{}", fmt_dur(e)));
    }
    json!({
        "stage": p.stage,
        "label": stage_label(p.stage),
        "step": p.step,
        "steps": p.steps,
        "done": p.done,
        "total": p.total,
        "unit": p.unit,
        "stage_percent": frac.map(|f| (f * 1000.0).round() / 10.0),
        "overall_percent": (overall * 10.0).round() / 10.0,
        "speed": tr.speed,
        "eta_secs": eta.map(|e| e.round()),
        "elapsed_secs": tr.started.elapsed().as_secs(),
        "elapsed": fmt_dur(tr.started.elapsed().as_secs_f64()),
        "eta": eta.map(fmt_dur),
        "detail": parts.join(" · "),
    })
}

pub struct AppState {
    pub data_dir: PathBuf,
    /// Serve the web UI from this folder instead of the copy built into the binary
    /// (the PC package ships it as `client/web`, so it can be edited without rebuilding).
    pub web_dir: Option<PathBuf>,
    pub catalog: RwLock<Option<Arc<Catalog>>>,
    pub shelves: RwLock<Shelves>,
    pub user: Mutex<Connection>,
    pub import: Mutex<JobStatus>,
    pub sources: RwLock<Sources>,
    /// Cached result of the last update check: (when, value)
    pub updates: Mutex<Option<(std::time::Instant, Value)>>,
    pub http: reqwest::Client,
    /// VNDB match index for external games: (catalog imported_at, index)
    pub ext_index: Mutex<Option<(String, Arc<vndb_core::external::matching::Index>)>>,
}

/// Open the personal database; a damaged file is kept as `user.sqlite.broken-<time>` and a new
/// one is created, so the app still starts.
fn open_user_or_recover(path: &std::path::Path) -> anyhow::Result<Connection> {
    match vndb_core::db::open_user(path) {
        Ok(c) => Ok(c),
        Err(e) if path.exists() => {
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            eprintln!(
                "user.sqlite cannot be opened ({e:#}); keeping it as user.sqlite.broken-{stamp}"
            );
            std::fs::rename(path, path.with_extension(format!("sqlite.broken-{stamp}")))?;
            vndb_core::db::open_user(path)
        }
        Err(e) => Err(e),
    }
}

/// Keep the image cache under `max` bytes: delete the least recently written files down to 3/4.
pub fn prune_image_cache(dir: &std::path::Path, max: u64) {
    let mut files = Vec::new();
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                stack.push(e.path());
            } else {
                total += m.len();
                let t = m.modified().unwrap_or(std::time::UNIX_EPOCH);
                files.push((t, m.len(), e.path()));
            }
        }
    }
    if total <= max {
        return;
    }
    files.sort();
    let target = max / 4 * 3;
    for (_, len, path) in files {
        if total <= target {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
    eprintln!("Image cache trimmed to {} MB", total / 1_048_576);
}

/// Image cache limit (covers, screenshots, tag examples).
pub const IMAGE_CACHE_MAX: u64 = 300 * 1024 * 1024;

impl AppState {
    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        let db = data_dir.join("catalog.sqlite");
        let catalog = if db.exists() {
            let t = std::time::Instant::now();
            match Catalog::open(&db) {
                Ok(c) => {
                    eprintln!(
                        "Loaded {} visual novels in {:.1?}",
                        c.vns.len(),
                        t.elapsed()
                    );
                    Some(Arc::new(c))
                }
                Err(e) => {
                    // a damaged database must not brick the app: put it aside and offer a new download
                    eprintln!("The catalog cannot be opened ({e:#}); it is moved to catalog.sqlite.broken");
                    let _ = std::fs::rename(&db, db.with_extension("sqlite.broken"));
                    None
                }
            }
        } else {
            eprintln!("No database yet: open the web UI and press \"Download database\", or run `vndb-web import`.");
            None
        };
        let img = data_dir.join("img");
        std::thread::spawn(move || prune_image_cache(&img, IMAGE_CACHE_MAX));
        Ok(Self {
            web_dir: None,
            shelves: RwLock::new(Shelves::load(&data_dir)?),
            user: Mutex::new(open_user_or_recover(&data_dir.join("user.sqlite"))?),
            catalog: RwLock::new(catalog),
            import: Mutex::new(JobStatus::default()),
            sources: RwLock::new(Sources::load(&data_dir)?),
            updates: Mutex::new(None),
            http: reqwest::Client::builder()
                .user_agent(vndb_core::http::user_agent())
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            ext_index: Mutex::new(None),
            data_dir,
        })
    }

    pub fn ready(&self) -> bool {
        self.catalog
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .is_some()
    }

    fn cat(&self) -> Result<Arc<Catalog>, ApiError> {
        self.catalog
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| {
                ApiError(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "database is not imported yet".into(),
                )
            })
    }

    fn marks(&self) -> HashMap<String, String> {
        let c = self.user.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(mut st) = c.prepare("SELECT vid, status FROM marks") else {
            return HashMap::new();
        };
        st.query_map([], |r| {
            Ok((format!("v{}", r.get::<_, u32>(0)?), r.get::<_, String>(1)?))
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default()
    }
}

/// Other web pages open in the same browser must not drive the local server (read or change
/// "My lists", start downloads): reject cross-site requests and DNS-rebinding hosts.
async fn same_origin_only(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let headers = req.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    let host_name = host.rsplit_once(':').map_or(host.as_str(), |(h, _)| h);
    // the UI is opened by IP or as localhost; a domain name pointing here is a rebinding attempt
    let host_ok = host_name == "localhost"
        || host_name.ends_with(".local")
        || (!host_name.is_empty() && !host_name.contains('.'))
        || host_name
            .trim_matches(|c| c == '[' || c == ']')
            .parse::<std::net::IpAddr>()
            .is_ok();
    let origin_ok = match headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        None => true,
        Some(o) => {
            let o = o.to_ascii_lowercase();
            o == format!("http://{host}") || o == format!("https://{host}")
        }
    };
    let cross_site = headers
        .get("sec-fetch-site")
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == "cross-site");
    if !host_ok || !origin_ok || (cross_site && req.uri().path().starts_with("/api/")) {
        return (StatusCode::FORBIDDEN, "cross-site request blocked").into_response();
    }
    next.run(req).await
}

pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({ "error": self.1 }))).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;
type St = State<Arc<AppState>>;

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, ApiError> + Send + 'static,
) -> Result<T, ApiError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/status", get(status))
        .route("/api/import", post(import))
        .route("/api/updates", get(updates))
        .route("/api/sync", post(sync))
        .route("/api/sources", get(sources_info))
        .route("/api/translate", post(translate))
        .route("/api/sources/refresh", post(sources_refresh))
        .route("/api/tags", get(tags))
        .route("/api/langs", get(langs))
        .route("/api/tags/:id", get(tag_detail))
        .route("/api/search", post(search))
        .route("/api/shelves", post(shelves_overview))
        .route("/api/shelves/:id", post(shelf_page))
        .route(
            "/api/shelves-config",
            get(shelves_config).put(save_shelves_config),
        )
        .route("/api/vn/:id", get(vn_detail))
        .route("/api/vn/:id/prices", get(vn_prices))
        .route("/api/vn/:id/similar", get(vn_similar))
        .route("/api/random", post(random))
        .route("/api/ext/status", get(external::status))
        .route("/api/ext/refresh", post(external::refresh))
        .route("/api/ext/search", post(external::search))
        .route("/api/ext/tags", get(external::tags))
        .route("/api/ext/game/:source/:id", get(external::game))
        .route("/api/ext/game/:source/:id/match", put(external::set_match))
        .route("/api/ext/audit", post(external::audit))
        .route("/api/ext/check", post(external::check))
        .route("/api/my", get(my_list))
        .route("/api/my/export", get(my_export))
        .route("/api/my/import", post(my_import))
        .route("/api/profile", get(profile).put(save_profile))
        .route("/api/my/:id", put(set_mark).delete(delete_mark))
        .route("/img/*path", get(image))
        .fallback(static_file)
        .layer(axum::middleware::from_fn(same_origin_only))
        // binaries are already compressed; compressing them again would also drop Content-Length
        .layer(
            tower_http::compression::CompressionLayer::new().compress_when(
                tower_http::compression::predicate::DefaultPredicate::new()
                    .and(
                        tower_http::compression::predicate::NotForContentType::const_new(
                            "application/vnd.android.package-archive",
                        ),
                    )
                    .and(
                        tower_http::compression::predicate::NotForContentType::const_new(
                            "application/octet-stream",
                        ),
                    ),
            ),
        )
        .with_state(state)
}

// ---------- status / import ----------

async fn status(State(s): St) -> Json<Value> {
    let imp = s
        .import
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let cat = s
        .catalog
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    Json(json!({
        "ready": cat.is_some(),
        "vns": cat.as_ref().map(|c| c.vns.len()),
        "tags": cat.as_ref().map(|c| c.tree.tags.len()),
        "dump_timestamp": cat.as_ref().map(|c| c.dump_timestamp.clone()),
        "lang_split": cat.as_ref().map(|c| c.lang_split),
        "imported_at": cat.as_ref().map(|c| c.imported_at.clone()),
        "import": imp,
        "version": vndb_core::app_version(),
    }))
}

/// Run `work` in a background thread as the single active job. `work` returns true when the
/// catalog file changed and must be reloaded.
pub fn start_job(
    state: Arc<AppState>,
    kind: &str,
    work: impl FnOnce(&AppState, &vndb_core::progress::Reporter) -> anyhow::Result<bool>
        + Send
        + 'static,
) -> bool {
    {
        let mut st = state.import.lock().unwrap_or_else(PoisonError::into_inner);
        if st.running {
            return false;
        }
        *st = JobStatus {
            kind: kind.to_string(),
            running: true,
            log: vec!["Starting".into()],
            error: None,
            finished: false,
            progress: None,
            track: Track::default(),
        };
    }
    std::thread::spawn(move || {
        let log = |m: &str| {
            eprintln!("{m}");
            let mut st = state.import.lock().unwrap_or_else(PoisonError::into_inner);
            st.log.push(m.to_string());
            let n = st.log.len();
            if n > 300 {
                st.log.drain(..n - 300);
            }
        };
        let prog = |p: &vndb_core::progress::Progress| {
            let mut st = state.import.lock().unwrap_or_else(PoisonError::into_inner);
            let kind = st.kind.clone();
            let mut tr = st.track.clone();
            st.progress = Some(progress_view(&kind, p, &mut tr));
            st.track = tr;
        };
        let rep = vndb_core::progress::Reporter::new(&log, &prog);
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&state, &rep)))
            .unwrap_or_else(|p| {
                let msg = p
                    .downcast_ref::<String>()
                    .cloned()
                    .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "unknown".into());
                Err(anyhow::anyhow!("внутренняя ошибка: {msg}"))
            });
        let res = res.and_then(|reload| {
            if reload {
                log("Loading catalog");
                let steps = vndb_core::progress::stages_of(
                    &state
                        .import
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .kind,
                )
                .len() as u32;
                rep.stage("load", steps, steps);
                let c = Catalog::open(&state.data_dir.join("catalog.sqlite"))?;
                *state
                    .catalog
                    .write()
                    .unwrap_or_else(PoisonError::into_inner) = Some(Arc::new(c));
            }
            Ok(())
        });
        *state.updates.lock().unwrap_or_else(PoisonError::into_inner) = None;
        let mut st = state.import.lock().unwrap_or_else(PoisonError::into_inner);
        st.running = false;
        st.finished = true;
        match res {
            Ok(()) => st.log.push("Ready".into()),
            Err(e) => st.error = Some(format!("{e:#}")),
        }
    });
    true
}

pub fn start_import(state: Arc<AppState>, file: Option<PathBuf>) -> bool {
    start_job(state, "import", move |st, log| {
        let db = st.data_dir.join("catalog.sqlite");
        let src = match &file {
            Some(p) => vndb_core::import::Source::File(p),
            None => vndb_core::import::Source::Url(vndb_core::import::DUMP_URL),
        };
        vndb_core::import::import(src, &db, log)?;
        // a database put aside as damaged is no longer needed
        let _ = std::fs::remove_file(db.with_extension("sqlite.broken"));
        Ok(true)
    })
}

pub fn start_sync(state: Arc<AppState>) -> bool {
    start_job(state, "sync", |st, log| {
        let r = vndb_core::sync::apply(&st.data_dir.join("catalog.sqlite"), log)?;
        Ok(r.new_vns + r.new_releases > 0)
    })
}

pub fn start_sources_refresh(state: Arc<AppState>) -> bool {
    start_job(state, "sources", |st, log| {
        let defs = st
            .sources
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        // separate connection: indexing must not block the UI's user database lock
        let conn = vndb_core::db::open_user(&st.data_dir.join("user.sqlite"))?;
        log.stage("sources", 1, 1);
        let res = vndb_core::sources::refresh_index(&defs, &conn, &|m: &str| log.log(m))?;
        for (id, n) in res {
            log.log(&format!("{id}: {n} pages"));
        }
        Ok(false)
    })
}

/// Refresh third-party indexes in the background when they are missing or older than a week.
pub fn maybe_refresh_sources(state: Arc<AppState>) {
    let stale = {
        let defs = state.sources.read().unwrap_or_else(PoisonError::into_inner);
        let user = state.user.lock().unwrap_or_else(PoisonError::into_inner);
        vndb_core::sources::index_age_days(&defs, &user)
            .map(|d| d > 7.0)
            .unwrap_or(true)
    };
    if stale {
        start_sources_refresh(state);
    }
}

async fn import(State(s): St) -> Json<Value> {
    // the HTTP API always downloads the official dump; local files are for the CLI only
    let started = start_import(s, None);
    Json(json!({ "started": started }))
}

#[derive(Deserialize)]
struct UpdatesQ {
    /// "1" / "true" force a fresh check (anything else uses the 6-hour cache)
    force: Option<String>,
}

/// How far behind VNDB the local database is (needs internet; cached for 6 hours).
async fn updates(State(s): St, Query(q): Query<UpdatesQ>) -> ApiResult<Value> {
    if !s.ready() {
        return Ok(Json(json!({ "ready": false })));
    }
    let force = matches!(q.force.as_deref(), Some("1" | "true" | "yes"));
    if !force {
        if let Some((at, v)) = s
            .updates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            if at.elapsed() < std::time::Duration::from_secs(6 * 3600) {
                return Ok(Json(v));
            }
        }
    }
    let db = s.data_dir.join("catalog.sqlite");
    let check = blocking(move || {
        Ok(vndb_core::sync::check(
            &db,
            &vndb_core::sync::Thresholds::default(),
        )?)
    })
    .await?;
    let v = json!({ "ready": true, "check": check });
    *s.updates.lock().unwrap_or_else(PoisonError::into_inner) =
        Some((std::time::Instant::now(), v.clone()));
    Ok(Json(v))
}

async fn sync(State(s): St) -> Json<Value> {
    Json(json!({ "started": start_sync(s) }))
}

async fn sources_info(State(s): St) -> ApiResult<Value> {
    let defs = s
        .sources
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
    let mut st = u
        .prepare("SELECT source, fetched_at, entries FROM source_meta")
        .map_err(anyhow::Error::from)?;
    let meta: HashMap<String, (String, i64)> = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                (r.get::<_, String>(1)?, r.get::<_, i64>(2)?),
            ))
        })
        .map_err(anyhow::Error::from)?
        .filter_map(Result::ok)
        .collect();
    let list: Vec<Value> = defs
        .sources
        .iter()
        .map(|d| {
            let m = meta.get(&d.id);
            json!({ "id": d.id, "name": d.name, "lang": d.lang, "kind": d.kind, "adult": d.adult, "description": d.description,
                    "indexed": d.index.is_some(), "fetched_at": m.map(|x| &x.0), "entries": m.map(|x| x.1) })
        })
        .collect();
    Ok(Json(json!({ "sources": list })))
}

#[derive(Deserialize)]
struct TranslateReq {
    texts: Vec<String>,
    #[serde(default = "default_lang")]
    to: String,
}

fn default_lang() -> String {
    "ru".into()
}

/// Machine translation (Google, MyMemory fallback), cached in user.sqlite. Needs internet
/// for texts that were never translated before.
async fn translate(State(s): St, Json(req): Json<TranslateReq>) -> ApiResult<Value> {
    if req.texts.len() > 400 || req.texts.iter().map(|t| t.len()).sum::<usize>() > 60_000 {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too much text".into(),
        ));
    }
    let path = s.data_dir.join("user.sqlite");
    let out = blocking(move || {
        let conn = vndb_core::db::open_user(&path)?;
        vndb_core::translate::translate(&conn, &req.texts, &req.to).map_err(|e| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("переводчик недоступен: {e:#}"),
            )
        })
    })
    .await?;
    Ok(Json(json!({ "translations": out })))
}

async fn sources_refresh(State(s): St) -> Json<Value> {
    Json(json!({ "started": start_sources_refresh(s) }))
}

// ---------- languages ----------

/// Every language VNs are available in, with the number of VNs (for the language filter).
async fn langs(State(s): St) -> ApiResult<Value> {
    let c = s.cat()?;
    // lang -> (any way, ready release, through a patch)
    let mut counts: HashMap<String, (u32, u32, u32)> = HashMap::new();
    for v in &c.vns {
        for (l, _) in &v.langs {
            let e = counts.entry(l.clone()).or_default();
            e.0 += 1;
            e.1 += v.ready_langs.contains(l) as u32;
            e.2 += v.patch_langs.contains(l) as u32;
        }
    }
    let mut list: Vec<(String, (u32, u32, u32))> = counts.into_iter().collect();
    list.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then(a.0.cmp(&b.0)));
    Ok(Json(json!({ "langs": list.into_iter()
        .map(|(lang, (vns, ready, patch))| json!({ "lang": lang, "vns": vns, "ready": ready, "patch": patch }))
        .collect::<Vec<_>>() })))
}

// ---------- tags ----------

#[derive(Serialize)]
struct TagOut<'a> {
    id: u32,
    name: &'a str,
    cat: &'a str,
    aliases: &'a [String],
    parents: &'a [u32],
    children: &'a [u32],
    vns: u32,
    searchable: bool,
    applicable: bool,
}

async fn tags(State(s): St) -> ApiResult<Value> {
    let c = s.cat()?;
    let mut list: Vec<TagOut> = c
        .tree
        .tags
        .values()
        .map(|t| TagOut {
            id: t.id,
            name: &t.name,
            cat: &t.cat,
            aliases: &t.aliases,
            parents: &t.parents,
            children: &t.children,
            vns: t.vns,
            searchable: t.searchable,
            applicable: t.applicable,
        })
        .collect();
    list.sort_by_key(|t| t.id);
    Ok(Json(json!({ "roots": c.tree.roots(), "tags": list })))
}

async fn tag_detail(State(s): St, Path(id): Path<String>) -> ApiResult<Value> {
    let c = s.cat()?;
    let id = c
        .tree
        .resolve(&id)
        .map_err(|e| ApiError(StatusCode::NOT_FOUND, e.to_string()))?;
    let t = c.tree.get(id).unwrap();
    let desc: String = c
        .connection()?
        .query_row("SELECT description FROM tags WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    let d = vndb_core::detail::parse_description(&desc);
    Ok(Json(json!({
        "id": id, "name": t.name, "cat": t.cat, "aliases": t.aliases, "vns": t.vns,
        "path": c.tree.path(id), "path_names": c.tree.path_names(id), "children": t.children,
        "description": d.text, "parts": d.parts, "examples": d.examples,
    })))
}

// ---------- search ----------

#[derive(Serialize)]
struct CardOut {
    #[serde(flatten)]
    card: Card,
    mark: Option<String>,
}

fn with_marks(items: Vec<Card>, marks: &HashMap<String, String>) -> Vec<CardOut> {
    items
        .into_iter()
        .map(|c| {
            let mark = marks.get(&c.id).cloned();
            CardOut { card: c, mark }
        })
        .collect()
}

async fn search(State(s): St, Json(req): Json<SearchRequest>) -> ApiResult<Value> {
    let c = s.cat()?;
    let marks = s.marks();
    let res = blocking(move || Ok(vndb_core::search(&c, &req)?)).await?;
    let mut v = serde_json::to_value(&res).unwrap();
    v["items"] = serde_json::to_value(with_marks(res.items, &marks)).unwrap();
    Ok(Json(v))
}

#[derive(Deserialize)]
#[serde(default)]
struct ShelvesReq {
    base: SearchRequest,
    per_shelf: usize,
    /// Show 18+ shelves
    adult: bool,
    sort: Option<Sort>,
    offset: usize,
    limit: usize,
}

impl Default for ShelvesReq {
    fn default() -> Self {
        Self {
            base: SearchRequest::default(),
            per_shelf: 20,
            adult: true,
            sort: None,
            offset: 0,
            limit: 48,
        }
    }
}

/// How many covers of a shelf row are visible without scrolling (wide screens show more).
const SHELF_FRONT: usize = 6;

async fn shelves_overview(State(s): St, body: Option<Json<ShelvesReq>>) -> ApiResult<Value> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let c = s.cat()?;
    let shelves = s
        .shelves
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone();
    let marks = s.marks();
    let out = blocking(move || {
        let per_shelf = req.per_shelf.clamp(1, 60);
        let mut base = req.base.clone();
        base.offset = 0;
        // extra candidates, so the same popular VNs do not open every shelf (see below)
        base.limit = (per_shelf * 3).min(120);
        let all: Vec<(usize, &vndb_core::shelves::Shelf)> = shelves
            .sections
            .iter()
            .enumerate()
            .filter(|(_, sec)| req.adult || !sec.adult)
            .flat_map(|(i, sec)| sec.shelves.iter().filter(|sh| req.adult || !sh.adult).map(move |sh| (i, sh)))
            .collect();
        // run shelves in parallel
        let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8);
        let results: Vec<Option<(usize, Value)>> = std::thread::scope(|scope| {
            let chunks: Vec<_> = all.chunks(all.len().div_ceil(threads).max(1)).collect();
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    let c = &c;
                    let base = &base;
                    let marks = &marks;
                    scope.spawn(move || {
                        chunk
                            .iter()
                            .map(|(sec, sh)| {
                                let r = shelves::run_shelf(c, sh, base).ok()?;
                                Some((*sec, json!({
                                    "id": sh.id, "title": sh.title, "adult": sh.adult, "total": r.total,
                                    "items": with_marks(r.items, marks), "warnings": r.warnings,
                                })))
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut sections: Vec<Value> = shelves
            .sections
            .iter()
            .filter(|sec| req.adult || !sec.adult)
            .map(|sec| json!({ "id": sec.id, "title": sec.title, "adult": sec.adult, "shelves": [] }))
            .collect();
        let idx: Vec<usize> = shelves.sections.iter().enumerate().filter(|(_, sec)| req.adult || !sec.adult).map(|(i, _)| i).collect();
        // Shelves come back in page order: a VN already shown near the start of an earlier shelf
        // moves behind the new ones, so each row opens with something different.
        let mut results: Vec<(usize, Value)> = results.into_iter().flatten().collect();
        let mut shown: HashSet<String> = HashSet::new();
        for (_, v) in results.iter_mut() {
            let items = v["items"].as_array().cloned().unwrap_or_default();
            let (fresh, seen): (Vec<Value>, Vec<Value>) = items
                .into_iter()
                .partition(|it| !shown.contains(it["id"].as_str().unwrap_or("")));
            let picked: Vec<Value> = fresh.into_iter().chain(seen).take(per_shelf).collect();
            for it in picked.iter().take(SHELF_FRONT) {
                shown.insert(it["id"].as_str().unwrap_or("").to_string());
            }
            v["items"] = Value::Array(picked);
        }
        for (sec, v) in results {
            if let Some(pos) = idx.iter().position(|i| *i == sec) {
                if v["total"].as_u64().unwrap_or(0) > 0 {
                    sections[pos]["shelves"].as_array_mut().unwrap().push(v);
                }
            }
        }
        Ok(json!({ "sections": sections }))
    })
    .await?;
    Ok(Json(out))
}

async fn shelf_page(
    State(s): St,
    Path(id): Path<String>,
    body: Option<Json<ShelvesReq>>,
) -> ApiResult<Value> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let c = s.cat()?;
    let shelf = s
        .shelves
        .read()
        .unwrap()
        .find(&id)
        .cloned()
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, format!("no shelf {id}")))?;
    let marks = s.marks();
    let out = blocking(move || {
        let mut r = shelves::request_for(&shelf, &req.base)?;
        if let Some(sort) = req.sort {
            r.sort = sort;
        }
        r.offset = req.offset;
        r.limit = req.limit;
        let res = vndb_core::search(&c, &r)?;
        let mut v = serde_json::to_value(&res).unwrap();
        v["items"] = serde_json::to_value(with_marks(res.items, &marks)).unwrap();
        v["shelf"] = json!({ "id": shelf.id, "title": shelf.title, "adult": shelf.adult, "query": shelf.query, "sort": r.sort });
        Ok(v)
    })
    .await?;
    Ok(Json(out))
}

async fn shelves_config(State(s): St) -> Json<Value> {
    Json(serde_json::to_value(&*s.shelves.read().unwrap_or_else(PoisonError::into_inner)).unwrap())
}

async fn save_shelves_config(State(s): St, Json(v): Json<Shelves>) -> ApiResult<Value> {
    let text = serde_json::to_string_pretty(&v).unwrap();
    std::fs::write(s.data_dir.join("shelves.json"), text)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    *s.shelves.write().unwrap_or_else(PoisonError::into_inner) = v;
    Ok(Json(json!({ "saved": true })))
}

async fn random(State(s): St, Json(req): Json<SearchRequest>) -> ApiResult<Value> {
    let c = s.cat()?;
    let out = blocking(move || {
        let mut r = req.clone();
        r.offset = 0;
        r.limit = 1;
        let total = vndb_core::search(&c, &r)?.total;
        if total == 0 {
            return Ok(json!({ "id": null }));
        }
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos() as usize;
        r.offset = nanos % total;
        let res = vndb_core::search(&c, &r)?;
        Ok(json!({ "id": res.items.first().map(|i| i.id.clone()) }))
    })
    .await?;
    Ok(Json(out))
}

// ---------- VN detail ----------

#[derive(Deserialize)]
struct DetailQ {
    langs: Option<String>,
    any_lang: Option<bool>,
}

fn parse_vid(id: &str) -> Result<u32, ApiError> {
    id.trim_start_matches(['v', 'V'])
        .parse()
        .map_err(|_| ApiError(StatusCode::BAD_REQUEST, "bad id".into()))
}

async fn vn_detail(
    State(s): St,
    Path(id): Path<String>,
    Query(q): Query<DetailQ>,
) -> ApiResult<Value> {
    let c = s.cat()?;
    let vid = parse_vid(&id)?;
    let order: Vec<String> = q
        .langs
        .unwrap_or_else(|| "en".into())
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    let any = q.any_lang.unwrap_or(false);
    let d = blocking(move || Ok(vndb_core::detail::detail(&c, vid, &order, any)?)).await?;
    let d = d.ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "no such VN".into()))?;
    let mut v = serde_json::to_value(&d).unwrap();
    let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
    let mark: Option<(String, Option<String>)> = u
        .query_row(
            "SELECT status, note FROM marks WHERE vid = ?1",
            [vid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    v["mark"] = json!(mark.as_ref().map(|m| &m.0));
    v["note"] = json!(mark.and_then(|m| m.1));
    v["external"] = external::for_vn(&s, vid);
    if let Some(vn) = s.cat()?.get(vid) {
        let defs = s.sources.read().unwrap_or_else(PoisonError::into_inner);
        let order: Vec<String> = v["langs"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l["lang"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        v["community"] = json!(vndb_core::sources::links_for(
            &defs,
            &u,
            vn,
            &d.aliases,
            &order,
            "community"
        ));
    }
    Ok(Json(v))
}

#[derive(Deserialize)]
struct SimilarQ {
    langs: Option<String>,
    /// "1": count sexual-content tags too
    adult: Option<String>,
    /// "1": leave out VNs with sexual content
    hide_ero: Option<String>,
}

/// Novels with the most similar tags (rarer tags weigh more).
async fn vn_similar(
    State(s): St,
    Path(id): Path<String>,
    Query(q): Query<SimilarQ>,
) -> ApiResult<Value> {
    let c = s.cat()?;
    let vid = parse_vid(&id)?;
    let order: Vec<String> = q
        .langs
        .unwrap_or_else(|| "en".into())
        .split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect();
    let opts = vndb_core::similar::Options {
        adult_tags: q.adult.as_deref() == Some("1"),
        hide_ero: q.hide_ero.as_deref() == Some("1"),
        ..Default::default()
    };
    let items = blocking(move || {
        let related = vndb_core::similar::relations(&c, vid);
        Ok(vndb_core::similar::similar(&c, vid, 16, &opts, &related)
            .into_iter()
            .filter_map(|(id, _)| c.get(id))
            .map(|v| vndb_core::search::card(v, &order, vec![], vec![]))
            .collect::<Vec<_>>())
    })
    .await?;
    Ok(Json(json!({ "items": items })))
}

#[derive(Deserialize)]
struct PriceQ {
    cc: Option<String>,
}

/// Current Steam prices for every Steam link of the VN (needs internet).
async fn vn_prices(
    State(s): St,
    Path(id): Path<String>,
    Query(q): Query<PriceQ>,
) -> ApiResult<Value> {
    let c = s.cat()?;
    let vid = parse_vid(&id)?;
    let cc = q.cc.unwrap_or_else(|| "us".into());
    let cc: String = cc
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .take(2)
        .collect();
    let urls: Vec<String> = blocking(move || {
        let conn = c.connection()?;
        let mut st = conn.prepare(
            "SELECT DISTINCT rl.url FROM release_vn rv JOIN release_links rl ON rl.rid = rv.rid WHERE rv.vid = ?1 AND rl.site = 'steam'
             UNION SELECT url FROM vn_links WHERE vid = ?1 AND site = 'steam'",
        ).map_err(anyhow::Error::from)?;
        let rows = st.query_map([vid], |r| r.get::<_, String>(0)).map_err(anyhow::Error::from)?;
        Ok(rows.filter_map(Result::ok).collect())
    })
    .await?;
    let mut out = Vec::new();
    for url in urls {
        let appid: String = url
            .split("/app/")
            .nth(1)
            .unwrap_or("")
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if appid.is_empty() {
            continue;
        }
        let api = format!("https://store.steampowered.com/api/appdetails?appids={appid}&cc={cc}&filters=basic,price_overview");
        let price = match s.http.get(api).send().await {
            Ok(r) => match r.json::<Value>().await {
                Ok(v) => {
                    let e = &v[&appid];
                    if !e["success"].as_bool().unwrap_or(false) {
                        "нет в вашем регионе".to_string()
                    } else if e["data"]["is_free"].as_bool().unwrap_or(false) {
                        "бесплатно".to_string()
                    } else if let Some(p) = e["data"]["price_overview"]["final_formatted"].as_str()
                    {
                        let d = e["data"]["price_overview"]["discount_percent"]
                            .as_u64()
                            .unwrap_or(0);
                        if d > 0 {
                            format!("{p} (-{d}%)")
                        } else {
                            p.to_string()
                        }
                    } else {
                        "цена не указана".to_string()
                    }
                }
                Err(_) => "цену не узнать".into(),
            },
            Err(_) => "нет связи".into(),
        };
        out.push(json!({ "url": url, "price": price }));
    }
    Ok(Json(json!({ "cc": cc, "prices": out })))
}

// ---------- personal shelves ----------

const STATUSES: &[&str] = &["wish", "playing", "finished", "dropped", "favorite"];

async fn my_list(State(s): St) -> ApiResult<Value> {
    let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
    let mut st = u
        .prepare("SELECT vid, status, note, updated FROM marks ORDER BY updated DESC")
        .map_err(anyhow::Error::from)?;
    let rows: Vec<Value> = st
        .query_map([], |r| {
            Ok(
                json!({ "id": format!("v{}", r.get::<_, u32>(0)?), "status": r.get::<_, String>(1)?,
                       "note": r.get::<_, Option<String>>(2)?, "updated": r.get::<_, String>(3)? }),
            )
        })
        .map_err(anyhow::Error::from)?
        .filter_map(Result::ok)
        .collect();
    Ok(Json(json!({ "statuses": STATUSES, "marks": rows })))
}

// ---------- profile (name, avatar, stats, achievements) ----------

fn user_setting(c: &Connection, key: &str) -> Option<String> {
    c.query_row("SELECT value FROM settings WHERE key = ?1", [key], |r| {
        r.get(0)
    })
    .ok()
}

async fn profile(State(s): St) -> ApiResult<Value> {
    let (name, avatar, marks) = {
        let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
        let mut st = u
            .prepare("SELECT vid, status FROM marks")
            .map_err(anyhow::Error::from)?;
        let marks: Vec<vndb_core::profile::Mark> = st
            .query_map([], |r| {
                Ok(vndb_core::profile::Mark {
                    vid: r.get(0)?,
                    status: r.get(1)?,
                })
            })
            .map_err(anyhow::Error::from)?
            .filter_map(Result::ok)
            .collect();
        (
            user_setting(&u, "profile_name"),
            user_setting(&u, "profile_avatar"),
            marks,
        )
    };
    let cat = s.cat()?;
    let year: u32 = cat
        .dump_timestamp
        .get(..4)
        .and_then(|y| y.parse().ok())
        .unwrap_or(2026);
    let p = blocking(move || Ok(vndb_core::profile::compute(&cat, &marks, year))).await?;
    Ok(Json(
        json!({ "name": name, "avatar": avatar, "stats": p.stats, "achievements": p.achievements }),
    ))
}

#[derive(Deserialize)]
struct ProfileReq {
    name: Option<String>,
    /// `data:image/...;base64,...` (resized on the device), or "" to remove
    avatar: Option<String>,
}

async fn save_profile(State(s): St, Json(req): Json<ProfileReq>) -> ApiResult<Value> {
    let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
    let set = |k: &str, v: &str| -> Result<(), ApiError> {
        if v.is_empty() {
            u.execute("DELETE FROM settings WHERE key = ?1", [k])
                .map_err(anyhow::Error::from)?;
        } else {
            u.execute(
                "INSERT OR REPLACE INTO settings(key, value) VALUES (?1, ?2)",
                [k, v],
            )
            .map_err(anyhow::Error::from)?;
        }
        Ok(())
    };
    if let Some(n) = &req.name {
        set(
            "profile_name",
            &n.trim().chars().take(40).collect::<String>(),
        )?;
    }
    if let Some(a) = &req.avatar {
        if !a.is_empty() && (!a.starts_with("data:image/") || a.len() > 400_000) {
            return Err(ApiError(
                StatusCode::BAD_REQUEST,
                "аватарка должна быть картинкой до 300 КБ".into(),
            ));
        }
        set("profile_avatar", a)?;
    }
    Ok(Json(json!({ "saved": true })))
}

#[derive(Deserialize)]
struct MarkReq {
    status: String,
    note: Option<String>,
    /// true: change only the status and keep the existing note (quick mark from a card)
    #[serde(default)]
    keep_note: bool,
}

async fn set_mark(
    State(s): St,
    Path(id): Path<String>,
    Json(m): Json<MarkReq>,
) -> ApiResult<Value> {
    let vid = parse_vid(&id)?;
    if !STATUSES.contains(&m.status.as_str()) {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            format!("status must be one of {STATUSES:?}"),
        ));
    }
    s.user
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO marks(vid, status, note, updated) VALUES (?1, ?2, ?3, datetime('now'))
             ON CONFLICT(vid) DO UPDATE SET status = excluded.status,
               note = CASE WHEN ?4 THEN marks.note ELSE excluded.note END, updated = excluded.updated",
            rusqlite::params![vid, m.status, m.note, m.keep_note],
        )
        .map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "ok": true })))
}

/// The personal shelves as a file (to move them to another device or keep a copy).
async fn my_export(State(s): St) -> Result<Response, ApiError> {
    let (marks, name) = {
        let u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
        let mut st = u
            .prepare("SELECT vid, status, note, updated FROM marks ORDER BY updated DESC")
            .map_err(anyhow::Error::from)?;
        let marks: Vec<Value> = st
            .query_map([], |r| {
                Ok(json!({ "id": format!("v{}", r.get::<_, u32>(0)?), "status": r.get::<_, String>(1)?,
                           "note": r.get::<_, Option<String>>(2)?, "updated": r.get::<_, Option<String>>(3)? }))
            })
            .map_err(anyhow::Error::from)?
            .filter_map(Result::ok)
            .collect();
        (marks, user_setting(&u, "profile_name"))
    };
    let body = json!({
        "format": "vnshelf-shelves", "format_version": 1, "app_version": vndb_core::app_version(),
        "profile": { "name": name }, "marks": marks,
    });
    let text = serde_json::to_string_pretty(&body).map_err(anyhow::Error::from)?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"vnshelf-shelves.json\"",
            ),
        ],
        text,
    )
        .into_response())
}

#[derive(Deserialize)]
struct ImportMark {
    id: String,
    status: String,
    note: Option<String>,
    updated: Option<String>,
}

#[derive(Deserialize)]
struct MyImport {
    format: Option<String>,
    marks: Vec<ImportMark>,
    /// true: the file replaces the shelves; false: merge (the newer mark of a novel wins)
    #[serde(default)]
    replace: bool,
}

async fn my_import(State(s): St, Json(req): Json<MyImport>) -> ApiResult<Value> {
    if req
        .format
        .as_deref()
        .is_some_and(|f| f != "vnshelf-shelves")
    {
        return Err(ApiError(
            StatusCode::BAD_REQUEST,
            "это не файл полок VN Shelf".into(),
        ));
    }
    if req.marks.len() > 100_000 {
        return Err(ApiError(
            StatusCode::PAYLOAD_TOO_LARGE,
            "too many marks".into(),
        ));
    }
    let mut u = s.user.lock().unwrap_or_else(PoisonError::into_inner);
    let tx = u.transaction().map_err(anyhow::Error::from)?;
    if req.replace {
        tx.execute("DELETE FROM marks", [])
            .map_err(anyhow::Error::from)?;
    }
    let (mut added, mut updated, mut skipped) = (0, 0, 0);
    for m in &req.marks {
        let Ok(vid) = parse_vid(&m.id) else {
            skipped += 1;
            continue;
        };
        if !STATUSES.contains(&m.status.as_str()) {
            skipped += 1;
            continue;
        }
        let when = m
            .updated
            .clone()
            .filter(|t| t.len() >= 10 && t.len() <= 32)
            .unwrap_or_else(|| "1970-01-01 00:00:00".into());
        let existing: Option<String> = tx
            .query_row("SELECT updated FROM marks WHERE vid = ?1", [vid], |r| {
                r.get(0)
            })
            .ok();
        match existing {
            // keep the newer of the two
            Some(cur) if cur >= when => skipped += 1,
            Some(_) => {
                tx.execute(
                    "UPDATE marks SET status = ?2, note = ?3, updated = ?4 WHERE vid = ?1",
                    rusqlite::params![vid, m.status, m.note, when],
                )
                .map_err(anyhow::Error::from)?;
                updated += 1;
            }
            None => {
                tx.execute(
                    "INSERT INTO marks(vid, status, note, updated) VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![vid, m.status, m.note, when],
                )
                .map_err(anyhow::Error::from)?;
                added += 1;
            }
        }
    }
    tx.commit().map_err(anyhow::Error::from)?;
    Ok(Json(
        json!({ "added": added, "updated": updated, "skipped": skipped }),
    ))
}

async fn delete_mark(State(s): St, Path(id): Path<String>) -> ApiResult<Value> {
    let vid = parse_vid(&id)?;
    s.user
        .lock()
        .unwrap()
        .execute("DELETE FROM marks WHERE vid = ?1", [vid])
        .map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "ok": true })))
}

// ---------- images (cached proxy for t.vndb.org) ----------

fn valid_image_path(p: &str) -> bool {
    let parts: Vec<&str> = p.split('/').collect();
    parts.len() == 3
        && ["cv", "cv.t", "sf", "st"].contains(&parts[0])
        && parts[1].len() == 2
        && parts[1].chars().all(|c| c.is_ascii_digit())
        && parts[2]
            .strip_suffix(".jpg")
            .map(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false)
}

async fn image(State(s): St, Path(path): Path<String>) -> Response {
    if !valid_image_path(&path) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let file = s.data_dir.join("img").join(&path);
    let bytes = match tokio::fs::read(&file).await {
        Ok(b) => b,
        Err(_) => {
            // Small (old) images have no thumbnail on VNDB: fall back to the full-size file.
            let full = path.replacen("cv.t/", "cv/", 1).replacen("st/", "sf/", 1);
            let mut candidates = vec![format!("https://t.vndb.org/{path}")];
            if full != path {
                candidates.push(format!("https://t.vndb.org/{full}"));
            }
            let mut found = None;
            let mut last = StatusCode::BAD_GATEWAY;
            for url in candidates {
                match s.http.get(&url).send().await {
                    Ok(r) if r.status().is_success() => {
                        found = Some(r);
                        break;
                    }
                    Ok(r) => {
                        last = StatusCode::from_u16(r.status().as_u16())
                            .unwrap_or(StatusCode::BAD_GATEWAY)
                    }
                    Err(_) => last = StatusCode::BAD_GATEWAY,
                }
            }
            let Some(resp) = found else {
                return last.into_response();
            };
            let Ok(b) = resp.bytes().await else {
                return StatusCode::BAD_GATEWAY.into_response();
            };
            if let Some(dir) = file.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            let _ = tokio::fs::write(&file, &b).await;
            // check the cache size now and then (every 300 new images)
            static WRITES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 300 == 299 {
                let dir = s.data_dir.join("img");
                tokio::task::spawn_blocking(move || prune_image_cache(&dir, IMAGE_CACHE_MAX));
            }
            b.to_vec()
        }
    };
    (
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("image/jpeg")),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=31536000, immutable"),
            ),
        ],
        bytes,
    )
        .into_response()
}

// ---------- static web UI ----------

/// File from the external web folder, if configured and present (no `..`, no absolute paths).
fn external_file(dir: &std::path::Path, path: &str) -> Option<Vec<u8>> {
    let rel = std::path::Path::new(path);
    if rel
        .components()
        .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return None;
    }
    std::fs::read(dir.join(rel)).ok()
}

async fn static_file(State(s): St, uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    let exists = |p: &str| {
        s.web_dir
            .as_deref()
            .map(|d| d.join(p).is_file())
            .unwrap_or(false)
            || Assets::get(p).is_some()
    };
    if path.is_empty() || !exists(&path) && !path.contains('.') {
        path = "index.html".into();
    }
    let data = s
        .web_dir
        .as_deref()
        .and_then(|d| external_file(d, &path))
        .or_else(|| Assets::get(&path).map(|f| f.data.into_owned()));
    match data {
        Some(bytes) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            // always revalidate: after an app update old cached modules must not mix with new ones
            let cache = if path.starts_with("icons/") {
                "public, max-age=86400"
            } else {
                "no-cache"
            };
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .body(Body::from(bytes))
                .unwrap()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_cache_is_trimmed_oldest_first() {
        let dir = std::env::temp_dir().join(format!("vnshelf-prune-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("cv/01")).unwrap();
        for i in 0..10 {
            std::fs::write(dir.join(format!("cv/01/{i}.jpg")), vec![0u8; 1000]).unwrap();
            let t =
                std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000 + i);
            std::fs::File::options()
                .write(true)
                .open(dir.join(format!("cv/01/{i}.jpg")))
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        prune_image_cache(&dir, 5000);
        let left: Vec<_> = (0..10)
            .filter(|i| dir.join(format!("cv/01/{i}.jpg")).exists())
            .collect();
        assert_eq!(left, vec![7, 8, 9]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn image_paths() {
        assert!(valid_image_path("cv.t/59/77859.jpg"));
        assert!(valid_image_path("st/07/5707.jpg"));
        assert!(!valid_image_path("../etc/passwd"));
        assert!(!valid_image_path("cv/59/../../x.jpg"));
        assert!(!valid_image_path("ch/12/12.jpg"));
    }

    #[test]
    fn external_web_dir_is_sandboxed() {
        let dir = std::env::temp_dir().join(format!("vnshelf-web-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("js")).unwrap();
        std::fs::write(dir.join("js/app.js"), "x").unwrap();
        assert_eq!(external_file(&dir, "js/app.js").as_deref(), Some(&b"x"[..]));
        assert!(external_file(&dir, "../etc/passwd").is_none());
        assert!(external_file(&dir, "/etc/passwd").is_none());
        let _ = std::fs::remove_dir_all(dir);
    }
}
