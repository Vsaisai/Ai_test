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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use vndb_core::search::{Card, SearchRequest, Sort};
use vndb_core::shelves::{self, Shelves};
use vndb_core::Catalog;

#[derive(RustEmbed)]
#[folder = "../../web/"]
struct Assets;

#[derive(Default, Clone, Serialize)]
pub struct ImportStatus {
    pub running: bool,
    pub log: Vec<String>,
    pub error: Option<String>,
}

pub struct AppState {
    pub data_dir: PathBuf,
    pub catalog: RwLock<Option<Arc<Catalog>>>,
    pub shelves: RwLock<Shelves>,
    pub user: Mutex<Connection>,
    pub import: Mutex<ImportStatus>,
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new(data_dir: PathBuf) -> anyhow::Result<Self> {
        let db = data_dir.join("catalog.sqlite");
        let catalog = if db.exists() {
            let t = std::time::Instant::now();
            let c = Catalog::open(&db)?;
            eprintln!(
                "Loaded {} visual novels in {:.1?}",
                c.vns.len(),
                t.elapsed()
            );
            Some(Arc::new(c))
        } else {
            eprintln!("No database yet: open the web UI and press \"Download database\", or run `vndb-web import`.");
            None
        };
        Ok(Self {
            shelves: RwLock::new(Shelves::load(&data_dir)?),
            user: Mutex::new(vndb_core::db::open_user(&data_dir.join("user.sqlite"))?),
            catalog: RwLock::new(catalog),
            import: Mutex::new(ImportStatus::default()),
            http: reqwest::Client::builder()
                .user_agent(concat!("vndb-explorer/", env!("CARGO_PKG_VERSION")))
                .timeout(std::time::Duration::from_secs(30))
                .build()?,
            data_dir,
        })
    }

    pub fn ready(&self) -> bool {
        self.catalog.read().unwrap().is_some()
    }

    fn cat(&self) -> Result<Arc<Catalog>, ApiError> {
        self.catalog.read().unwrap().clone().ok_or_else(|| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "database is not imported yet".into(),
            )
        })
    }

    fn marks(&self) -> HashMap<String, String> {
        let c = self.user.lock().unwrap();
        let mut st = c.prepare("SELECT vid, status FROM marks").unwrap();
        st.query_map([], |r| {
            Ok((format!("v{}", r.get::<_, u32>(0)?), r.get::<_, String>(1)?))
        })
        .unwrap()
        .filter_map(Result::ok)
        .collect()
    }
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
        .route("/api/tags", get(tags))
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
        .route("/api/random", post(random))
        .route("/api/my", get(my_list))
        .route("/api/my/:id", put(set_mark).delete(delete_mark))
        .route("/img/*path", get(image))
        .fallback(static_file)
        .layer(tower_http::cors::CorsLayer::permissive())
        .layer(tower_http::compression::CompressionLayer::new())
        .with_state(state)
}

// ---------- status / import ----------

async fn status(State(s): St) -> Json<Value> {
    let imp = s.import.lock().unwrap().clone();
    let cat = s.catalog.read().unwrap().clone();
    Json(json!({
        "ready": cat.is_some(),
        "vns": cat.as_ref().map(|c| c.vns.len()),
        "tags": cat.as_ref().map(|c| c.tree.tags.len()),
        "dump_timestamp": cat.as_ref().map(|c| c.dump_timestamp.clone()),
        "imported_at": cat.as_ref().map(|c| c.imported_at.clone()),
        "import": imp,
        "version": env!("CARGO_PKG_VERSION"),
    }))
}

#[derive(Deserialize, Default)]
struct ImportReq {
    file: Option<String>,
}

pub fn start_import(state: Arc<AppState>, file: Option<PathBuf>) -> bool {
    {
        let mut st = state.import.lock().unwrap();
        if st.running {
            return false;
        }
        *st = ImportStatus {
            running: true,
            log: vec!["Starting".into()],
            error: None,
        };
    }
    std::thread::spawn(move || {
        let db = state.data_dir.join("catalog.sqlite");
        let log = |m: &str| {
            eprintln!("{m}");
            state.import.lock().unwrap().log.push(m.to_string());
        };
        let src = match &file {
            Some(p) => vndb_core::import::Source::File(p),
            None => vndb_core::import::Source::Url(vndb_core::import::DUMP_URL),
        };
        let res = vndb_core::import::import(src, &db, &log).and_then(|_| {
            log("Loading catalog");
            Catalog::open(&db)
        });
        let mut st = state.import.lock().unwrap();
        st.running = false;
        match res {
            Ok(c) => {
                *state.catalog.write().unwrap() = Some(Arc::new(c));
                st.log.push("Ready".into());
            }
            Err(e) => st.error = Some(format!("{e:#}")),
        }
    });
    true
}

async fn import(State(s): St, body: Option<Json<ImportReq>>) -> Json<Value> {
    let file = body.and_then(|b| b.0.file).map(PathBuf::from);
    let started = start_import(s, file);
    Json(json!({ "started": started }))
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
    Ok(Json(json!({
        "id": id, "name": t.name, "cat": t.cat, "aliases": t.aliases, "vns": t.vns,
        "path": c.tree.path(id), "path_names": c.tree.path_names(id), "children": t.children,
        "description": vndb_core::detail::clean_bbcode(&desc),
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

async fn shelves_overview(State(s): St, body: Option<Json<ShelvesReq>>) -> ApiResult<Value> {
    let req = body.map(|b| b.0).unwrap_or_default();
    let c = s.cat()?;
    let shelves = s.shelves.read().unwrap().clone();
    let marks = s.marks();
    let out = blocking(move || {
        let mut base = req.base.clone();
        base.offset = 0;
        base.limit = req.per_shelf.clamp(1, 60);
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
        for (sec, v) in results.into_iter().flatten() {
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
    Json(serde_json::to_value(&*s.shelves.read().unwrap()).unwrap())
}

async fn save_shelves_config(State(s): St, Json(v): Json<Shelves>) -> ApiResult<Value> {
    let text = serde_json::to_string_pretty(&v).unwrap();
    std::fs::write(s.data_dir.join("shelves.json"), text)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    *s.shelves.write().unwrap() = v;
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
    let u = s.user.lock().unwrap();
    let mark: Option<(String, Option<String>)> = u
        .query_row(
            "SELECT status, note FROM marks WHERE vid = ?1",
            [vid],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .ok();
    v["mark"] = json!(mark.as_ref().map(|m| &m.0));
    v["note"] = json!(mark.and_then(|m| m.1));
    Ok(Json(v))
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
                        "unavailable".to_string()
                    } else if e["data"]["is_free"].as_bool().unwrap_or(false) {
                        "free".to_string()
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
                        "no price".to_string()
                    }
                }
                Err(_) => "?".into(),
            },
            Err(_) => "offline".into(),
        };
        out.push(json!({ "url": url, "price": price }));
    }
    Ok(Json(json!({ "cc": cc, "prices": out })))
}

// ---------- personal shelves ----------

const STATUSES: &[&str] = &["wish", "playing", "finished", "dropped", "favorite"];

async fn my_list(State(s): St) -> ApiResult<Value> {
    let u = s.user.lock().unwrap();
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

#[derive(Deserialize)]
struct MarkReq {
    status: String,
    note: Option<String>,
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
             ON CONFLICT(vid) DO UPDATE SET status = excluded.status, note = excluded.note, updated = excluded.updated",
            rusqlite::params![vid, m.status, m.note],
        )
        .map_err(anyhow::Error::from)?;
    Ok(Json(json!({ "ok": true })))
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

async fn static_file(uri: Uri) -> Response {
    let mut path = uri.path().trim_start_matches('/').to_string();
    if path.is_empty() || Assets::get(&path).is_none() && !path.contains('.') {
        path = "index.html".into();
    }
    match Assets::get(&path) {
        Some(f) => {
            let mime = mime_guess::from_path(&path).first_or_octet_stream();
            let cache = if path == "index.html" || path == "sw.js" {
                "no-cache"
            } else {
                "public, max-age=3600"
            };
            Response::builder()
                .header(header::CONTENT_TYPE, mime.as_ref())
                .header(header::CACHE_CONTROL, cache)
                .body(Body::from(f.data.into_owned()))
                .unwrap()
        }
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn image_paths() {
        assert!(valid_image_path("cv.t/59/77859.jpg"));
        assert!(valid_image_path("st/07/5707.jpg"));
        assert!(!valid_image_path("../etc/passwd"));
        assert!(!valid_image_path("cv/59/../../x.jpg"));
        assert!(!valid_image_path("ch/12/12.jpg"));
    }
}
