//! Reading the "Latest updates" JSON feed (`/sam/latest_alpha/latest_data.php`): the tag and
//! prefix dictionaries, one list page, and the polite request pace every F95Zone read shares.

use super::{check_response, ID};
use crate::external::{ExtGame, ListPage, ProviderError};
use serde_json::Value;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Snapshot of the public tag ids, used when the live dictionary cannot be read.
const FALLBACK_TAGS: &str = include_str!("../../../assets/f95_tags.json");

/// One page of `latest_data.php`.
pub fn list(
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
                .map(|s| super::html::unescape(s).trim().to_string())
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

/// Exact size of a paged list from its first and last page.
pub(super) fn size(per_page: usize, last_page: u32, on_last: usize) -> usize {
    per_page * (last_page.saturating_sub(1) as usize) + on_last
}
}

/// Tag names and prefix names by id, read from the "Latest updates" page.
pub type Dictionaries = (HashMap<u64, String>, HashMap<u64, String>);

/// `var latestUpdates = {...};` on the "Latest updates" page: `tags` is `{id: name}`; prefixes
/// are objects `{id, name}` somewhere under a `prefixes` key.
pub fn dictionaries(page: &str) -> Result<Dictionaries, ProviderError> {
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

/// Numbers in the feed arrive as integers, floats or strings; anything else is missing.
fn num(v: &Value) -> Option<u64> {
    v.as_u64()
        .or_else(|| v.as_f64().map(|f| f as u64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// Snapshot of the public tag ids shipped with the app, so most tags keep their names even
/// when the live dictionary page cannot be read.
pub fn fallback_tags() -> HashMap<u64, String> {
    serde_json::from_str::<HashMap<String, String>>(FALLBACK_TAGS)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
        .collect()
}

// ---------- the shared request pace ----------

static LAST_REQUEST: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// Wait until the provider's current `min_gap` has passed since the last request anywhere in
/// the process, then mark this one. The import job and pages opened by the user share one pace.
pub(super) fn pace(min_gap: Duration) {
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
}
