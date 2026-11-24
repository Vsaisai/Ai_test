//! `/api/ext/*`: games from external catalogs (F95Zone), metadata only.

use super::{blocking, start_job, ApiError, ApiResult, AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::{Arc, PoisonError};
use vndb_core::external::f95zone::{self, F95Zone};
use vndb_core::external::matching::Index;
use vndb_core::external::{self as ext, store, Provider};

const SOURCES: &[&str] = &[f95zone::ID];
/// A game page is read again after a week (version, description and links change).
const DETAILS_MAX_AGE: u64 = 7 * 24 * 3600;

fn db(s: &AppState) -> anyhow::Result<rusqlite::Connection> {
    store::open(&s.data_dir.join("external.sqlite"))
}

fn source_ok(src: &str) -> Result<(), ApiError> {
    if SOURCES.contains(&src) {
        Ok(())
    } else {
        Err(ApiError(StatusCode::NOT_FOUND, "unknown source".into()))
    }
}

/// VNDB match index, rebuilt when the catalog changes.
fn index(s: &AppState) -> Option<Arc<Index>> {
    let cat = s
        .catalog
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    let mut slot = s.ext_index.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((stamp, idx)) = slot.as_ref() {
        if stamp.ptr_eq(&Arc::downgrade(&cat)) {
            return Some(idx.clone());
        }
    }
    match Index::build(&cat) {
        Ok(idx) => {
            let idx = Arc::new(idx);
            *slot = Some((Arc::downgrade(&cat), idx.clone()));
            Some(idx)
        }
        Err(e) => {
            eprintln!("external: cannot build the VNDB index: {e:#}");
            None
        }
    }
}

pub async fn status(State(s): St) -> ApiResult<Value> {
    let out = blocking(move || {
        let conn = db(&s)?;
        let mut v = json!({});
        for src in SOURCES {
            v[*src] = json!(store::stats(&conn, src)?);
            if let Some(t) =
                store::get_meta(&conn, src, "cooldown_until")?.and_then(|t| t.parse::<u64>().ok())
            {
                f95zone::set_cooldown_until(t);
            }
            v[*src]["cooldown_left"] = json!(f95zone::cooldown_left());
            v[*src]["last_check"] = store::get_meta(&conn, src, "last_check")?
                .and_then(|c| serde_json::from_str::<Value>(&c).ok())
                .unwrap_or(Value::Null);
        }
        Ok(v)
    })
    .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct RefreshReq {
    #[serde(default = "f95")]
    source: String,
    /// Read every page instead of stopping at the first unchanged one
    #[serde(default)]
    full: bool,
}

fn f95() -> String {
    f95zone::ID.into()
}

fn provider(source: &str) -> anyhow::Result<Box<dyn Provider>> {
    match source {
        f95zone::ID => Ok(Box::new(F95Zone::new())),
        other => anyhow::bail!("unknown source {other}"),
    }
}

/// Re-match the catalog with VNDB and put the result into the saved report.
fn rematch(
    st: &AppState,
    conn: &rusqlite::Connection,
    source: &str,
    log: &dyn Fn(&str),
) -> anyhow::Result<()> {
    if let Some(idx) = index(st) {
        vndb_core::external::matching::match_all(conn, &idx)?;
    }
    let stats = store::stats(conn, source)?;
    log(&format!("Есть в VNDB: {}", stats.in_vndb));
    if let Some(mut rep) = store::get_meta(conn, source, "last_report")?
        .and_then(|r| serde_json::from_str::<ext::RefreshReport>(&r).ok())
    {
        rep.in_vndb = Some(stats.in_vndb);
        rep.duplicates = stats.duplicates;
        store::set_meta(conn, source, "last_report", &serde_json::to_string(&rep)?)?;
    }
    Ok(())
}

pub(super) fn match_reloaded_catalog(s: &AppState, log: &dyn Fn(&str)) -> anyhow::Result<()> {
    if !s.data_dir.join("external.sqlite").is_file() { return Ok(()); }
    let conn = db(s)?;
    if conn.query_row("SELECT EXISTS(SELECT 1 FROM ext_games)", [], |r| r.get::<_,bool>(0))? {
        rematch(s, &conn, f95zone::ID, log)?;
    }
    Ok(())
}

/// The callbacks of a run, wired to the job's log, its progress and the Stop button.
/// `stages` is the job's stage list (see `stages_of`): progress of a stage shows its position.
fn with_run<T>(
    st: &AppState,
    log: &vndb_core::progress::Reporter,
    stages: &'static [&'static str],
    f: impl FnOnce(&ext::Run) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let run = ext::Run {
        log: &|m: &str| log.log(m),
        progress: &|stage, done, total| {
            let step = stages.iter().position(|s| *s == stage).unwrap_or(0) as u32 + 1;
            log.progress(vndb_core::progress::Progress {
                stage,
                step,
                steps: stages.len() as u32,
                done,
                total,
                unit: "items",
            })
        },
        stop: &|| st.cancel.load(std::sync::atomic::Ordering::Relaxed),
    };
    f(&run)
}

pub fn start_refresh(state: Arc<AppState>, source: &str, full: bool) -> bool {
    let source = source.to_string();
    start_job(state, "external", move |st, log| {
        let conn = db(st)?;
        let stages = vndb_core::progress::stages_of("external");
        log.stage("external", 1, stages.len() as u32);
        let mut p = provider(&source)?;
        // 5000 pages is only a safety limit: the feed's own page count decides
        let rep = with_run(st, log, stages, |run| {
            ext::refresh(p.as_mut(), &conn, full, 5000, run)
        })?;
        log.log(&format!(
            "Найдено релизов: {}, новых: {}, обновлённых: {}, без изменений: {}, скрыто правилами: {}, ошибок: {}, повторов: {}",
            rep.discovered, rep.new, rep.updated, rep.unchanged, rep.hidden_by_policy, rep.errors, rep.repeats
        ));
        log.stage("matching", stages.len() as u32, stages.len() as u32);
        rematch(st, &conn, &source, &|m: &str| log.log(m))?;
        if rep.cancelled {
            log.log("Остановлено. Следующий запуск продолжит с того же места.");
        } else if let Some(why) = rep.stopped {
            // what was read is kept; the job reports the reason
            anyhow::bail!("{why}");
        }
        Ok(false)
    })
}

#[derive(Deserialize)]
pub struct AuditReq {
    #[serde(default = "f95")]
    source: String,
    /// Thread pages to read per run for threads found only in the forum (tag check)
    #[serde(default = "check_limit")]
    check_limit: usize,
}

fn check_limit() -> usize {
    // all of them: the stage shows its progress, can be stopped and goes on next time
    5000
}

pub async fn audit(State(s): St, Json(req): Json<AuditReq>) -> ApiResult<Value> {
    source_ok(&req.source)?;
    let source = req.source.clone();
    let limit = req.check_limit.min(5000);
    let started = start_job(s, "external-audit", move |st, log| {
        let conn = db(st)?;
        let stages = vndb_core::progress::stages_of("external-audit");
        log.stage("external-audit", 1, stages.len() as u32);
        let mut p = provider(&source)?;
        let rep = with_run(st, log, stages, |run| {
            ext::audit(p.as_mut(), &conn, 20_000, limit, run)
        })?;
        log.log(&format!(
            "Тем в форуме: {}, уже были в каталоге: {}, не было в ленте: {}, прочитано страниц тем: {}, осталось прочитать: {}, ошибок: {}",
            rep.index_threads, rep.already_in_catalog, rep.missing_added, rep.checked, rep.pending, rep.errors
        ));
        log.stage("matching", stages.len() as u32, stages.len() as u32);
        rematch(st, &conn, &source, &|m: &str| log.log(m))?;
        if rep.cancelled {
            log.log("Остановлено. Следующий запуск продолжит с того же места.");
        } else if let Some(why) = rep.stopped {
            anyhow::bail!("{why}");
        }
        Ok(false)
    });
    Ok(Json(json!({ "started": started })))
}

/// Ask the running job to stop after its current request (F95Zone jobs listen to it).
pub async fn stop(State(s): St) -> ApiResult<Value> {
    let (running, kind) = {
        let j = s.import.lock().unwrap_or_else(PoisonError::into_inner);
        (j.running, j.kind.clone())
    };
    let ours = running && kind.starts_with("external");
    if ours {
        s.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(Json(json!({ "stopping": ours })))
}

/// The short test against the real site (about 7 requests).
pub async fn check(State(s): St) -> ApiResult<Value> {
    let started = start_job(s, "external-check", move |st, log| {
        let conn = db(st)?;
        log.stage("external-check", 1, 1);
        let r = f95zone::live_check(&conn, 3, &|m: &str| log.log(m))?;
        if let Some(why) = r.stopped {
            anyhow::bail!("{why}");
        }
        Ok(false)
    });
    Ok(Json(json!({ "started": started })))
}

#[derive(Deserialize)]
pub struct MatchReq {
    /// VNDB id ("v123", "123" or a vndb.org address); null = "this game is not in VNDB"
    vndb: Option<String>,
    /// true: forget the manual decision and match automatically again
    #[serde(default)]
    auto: bool,
}

fn parse_vndb(s: &str) -> Option<u32> {
    let s = s.trim().trim_end_matches('/');
    let tail = s.rsplit('/').next().unwrap_or(s);
    tail.trim_start_matches(['v', 'V']).parse().ok()
}

/// The user's correction of a VNDB match.
pub async fn set_match(
    State(s): St,
    Path((src, id)): Path<(String, String)>,
    Json(req): Json<MatchReq>,
) -> ApiResult<Value> {
    source_ok(&src)?;
    let out = blocking(move || {
        let conn = db(&s)?;
        if req.auto {
            store::clear_manual_match(&conn, &src, &id)?;
            if let (Some(idx), Some(st)) = (index(&s), store::get(&conn, &src, &id)?) {
                let m = idx.find(&st.game);
                store::set_match(&conn, &src, &id, m.map(|x| x.0), m.map(|x| x.1))?;
            }
        } else {
            let vid = match req.vndb.as_deref().filter(|v| !v.trim().is_empty()) {
                None => None,
                Some(v) => {
                    let vid = parse_vndb(v).ok_or_else(|| {
                        ApiError(StatusCode::BAD_REQUEST, "номер VNDB вида v1234".into())
                    })?;
                    let cat = s.catalog.read().unwrap_or_else(PoisonError::into_inner).clone();
                    if cat.as_ref().is_some_and(|c| c.get(vid).is_none()) {
                        return Err(ApiError(
                            StatusCode::NOT_FOUND,
                            format!("в базе нет новеллы v{vid}"),
                        ));
                    }
                    Some(vid)
                }
            };
            if !store::set_manual_match(&conn, &src, &id, vid)? {
                return Err(ApiError(StatusCode::NOT_FOUND, "not available".into()));
            }
        }
        let g = store::get(&conn, &src, &id)?;
        Ok(json!({ "vndb_id": g.as_ref().and_then(|g| g.vndb_id), "match_kind": g.as_ref().and_then(|g| g.match_kind.clone()),
                   "match_manual": g.map(|g| g.match_manual) }))
    })
    .await?;
    Ok(Json(out))
}

pub async fn refresh(State(s): St, Json(req): Json<RefreshReq>) -> ApiResult<Value> {
    source_ok(&req.source)?;
    Ok(Json(
        json!({ "started": start_refresh(s, &req.source, req.full) }),
    ))
}

pub async fn search(State(s): St, Json(q): Json<store::Query>) -> ApiResult<Value> {
    let out = blocking(move || {
        let conn = db(&s)?;
        let page = store::search(&conn, &q)?;
        let cat = s
            .catalog
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let mut v = json!(page);
        // title of the VNDB entry a game was merged with
        if let (Some(cat), Some(items)) = (cat, v["items"].as_array_mut()) {
            for it in items {
                if let Some(vn) = it["vndb_id"].as_u64().and_then(|id| cat.get(id as u32)) {
                    it["vndb_title"] = json!(vn.display_title(&[]));
                }
            }
        }
        Ok(v)
    })
    .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct TagsQ {
    source: Option<String>,
}

pub async fn tags(State(s): St, Query(q): Query<TagsQ>) -> ApiResult<Value> {
    let src = q.source.unwrap_or_else(f95);
    source_ok(&src)?;
    let out = blocking(move || {
        let conn = db(&s)?;
        let tags: Vec<Value> = store::tag_counts(&conn, &src)?
            .into_iter()
            .take(400)
            .map(|(t, n)| json!({ "tag": t, "games": n }))
            .collect();
        Ok(json!({ "tags": tags }))
    })
    .await?;
    Ok(Json(out))
}

#[derive(Deserialize)]
pub struct GameQ {
    /// "1": read the game page again now
    refresh: Option<String>,
}

/// One game; its page is read (once a week at most) for the description and official links.
pub async fn game(
    State(s): St,
    Path((src, id)): Path<(String, String)>,
    Query(q): Query<GameQ>,
) -> ApiResult<Value> {
    source_ok(&src)?;
    let out = blocking(move || {
        let conn = db(&s)?;
        if store::is_blocked(&conn, &src, &id)? {
            return Err(ApiError(StatusCode::NOT_FOUND, "not available".into()));
        }
        let cached = store::get(&conn, &src, &id)?;
        let stale = cached
            .as_ref()
            .map(|c| store::age_secs(&c.details_at).is_none_or(|a| a > DETAILS_MAX_AGE))
            .unwrap_or(true);
        let mut details_error = None;
        if stale || q.refresh.as_deref() == Some("1") {
            let mut p = F95Zone::new();
            // a pause the site asked for also holds for pages the user opens
            ext::load_cooldown(&mut p, &conn)?;
            match p.details(&id) {
                Ok(g) => {
                    store::save_details(&conn, &g)?;
                    // hidden by the policy after reading the page: not shown
                    if let (Some(idx), Some(st)) = (index(&s), store::get(&conn, &src, &id)?) {
                        let m = idx.find(&st.game);
                        store::set_match(&conn, &src, &id, m.map(|x| x.0), m.map(|x| x.1))?;
                    }
                }
                Err(e) => details_error = Some(e.to_string()),
            }
        }
        let Some(game) = store::get(&conn, &src, &id)? else {
            return Err(ApiError(
                StatusCode::NOT_FOUND,
                details_error.unwrap_or_else(|| "not available".into()),
            ));
        };
        let mut v = json!(game);
        let key = super::shelf_key(&format!("f95-{id}"))?;
        use rusqlite::OptionalExtension;
        let mark: Option<String> = s.user.lock().unwrap_or_else(PoisonError::into_inner)
            .query_row("SELECT status FROM marks WHERE vid = ?1", [key], |r| r.get(0))
            .optional().map_err(anyhow::Error::from)?;
        v["mark"] = json!(mark);
        v["details_error"] = json!(details_error);
        if let Some(vid) = game.vndb_id {
            let cat = s
                .catalog
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(vn) = cat.as_ref().and_then(|c| c.get(vid)) {
                v["vndb_title"] = json!(vn.display_title(&[]));
            }
        }
        Ok(v)
    })
    .await?;
    Ok(Json(out))
}

/// External pages of a VNDB entry (for the VN page).
pub fn for_vn(s: &AppState, vid: u32) -> Value {
    let Ok(conn) = db(s) else {
        return json!([]);
    };
    let list: Vec<Value> = store::for_vndb(&conn, vid)
        .unwrap_or_default()
        .into_iter()
        .map(|g| {
            json!({ "source": g.game.source, "ext_id": g.game.ext_id, "name": "F95Zone",
                    "url": g.game.page_url, "version": g.game.version, "status": g.game.status,
                    "updated": g.game.updated, "match": g.match_kind, "manual": g.match_manual })
        })
        .collect();
    json!(list)
}
