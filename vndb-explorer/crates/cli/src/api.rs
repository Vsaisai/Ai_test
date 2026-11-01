//! Thin client for the public VNDB Kana API (https://api.vndb.org/kana).

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};
use std::thread::sleep;
use std::time::Duration;

const API: &str = "https://api.vndb.org/kana";

pub struct Client {
    http: reqwest::blocking::Client,
}

#[derive(Deserialize)]
struct Page<T> {
    results: Vec<T>,
    #[serde(default)]
    more: bool,
    #[serde(default)]
    count: Option<u64>,
}

/// Result of a (possibly multi-page) query.
pub struct Fetched<T> {
    pub items: Vec<T>,
    /// Total number of matches reported by VNDB.
    pub total: Option<u64>,
}

impl Client {
    pub fn new() -> Result<Self> {
        let http = reqwest::blocking::Client::builder()
            .user_agent(concat!("vndb-explorer/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self { http })
    }

    pub fn http(&self) -> &reqwest::blocking::Client {
        &self.http
    }

    fn post(&self, endpoint: &str, body: &Value) -> Result<String> {
        let url = format!("{API}/{endpoint}");
        // VNDB throttles (HTTP 429); back off a few times before giving up.
        for attempt in 0..5u64 {
            let resp = self
                .http
                .post(&url)
                .json(body)
                .send()
                .with_context(|| format!("request to {url} failed"))?;
            let status = resp.status();
            let text = resp.text()?;
            if status.as_u16() == 429 {
                sleep(Duration::from_secs(2 + attempt * 2));
                continue;
            }
            if !status.is_success() {
                bail!("VNDB API error {status} on /{endpoint}: {}", text.trim());
            }
            return Ok(text);
        }
        bail!("VNDB API kept throttling requests (HTTP 429), try again later")
    }

    /// Run a query and follow pagination until `max` items are collected.
    pub fn query<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        filters: Value,
        fields: &str,
        sort: Option<(&str, bool)>,
        max: usize,
    ) -> Result<Fetched<T>> {
        let mut items = Vec::new();
        let mut total = None;
        let mut page = 1;
        loop {
            let want = (max - items.len()).clamp(1, 100);
            let mut body = json!({
                "filters": filters,
                "fields": fields,
                "results": want,
                "page": page,
                "count": page == 1,
            });
            if let Some((key, reverse)) = sort {
                body["sort"] = json!(key);
                body["reverse"] = json!(reverse);
            }
            let text = self.post(endpoint, &body)?;
            let p: Page<T> = serde_json::from_str(&text)
                .with_context(|| format!("unexpected VNDB response from /{endpoint}"))?;
            if p.count.is_some() {
                total = p.count;
            }
            items.extend(p.results);
            if !p.more || items.len() >= max {
                break;
            }
            page += 1;
        }
        Ok(Fetched { items, total })
    }
}

// ---------- Response models ----------

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Title {
    pub lang: String,
    pub title: String,
    pub latin: Option<String>,
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub main: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct VnTag {
    pub id: String,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub spoiler: u8,
    #[serde(default)]
    pub lie: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ExtLink {
    pub url: String,
    pub label: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Producer {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub developer: bool,
    #[serde(default)]
    pub publisher: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Developer {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Vn {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub alttitle: Option<String>,
    #[serde(default)]
    pub olang: Option<String>,
    #[serde(default)]
    pub titles: Vec<Title>,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub rating: Option<f64>,
    #[serde(default)]
    pub votecount: Option<u64>,
    #[serde(default)]
    pub length_minutes: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<VnTag>,
    #[serde(default)]
    pub developers: Vec<Developer>,
    #[serde(default)]
    pub extlinks: Vec<ExtLink>,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct ReleaseLang {
    pub lang: String,
    #[serde(default)]
    pub mtl: bool,
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub struct Release {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub released: Option<String>,
    #[serde(default)]
    pub languages: Vec<ReleaseLang>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default)]
    pub freeware: bool,
    #[serde(default)]
    pub official: bool,
    #[serde(default)]
    pub patch: bool,
    #[serde(default)]
    pub extlinks: Vec<ExtLink>,
    #[serde(default)]
    pub producers: Vec<Producer>,
}

pub const VN_LIST_FIELDS: &str = "title,alttitle,olang,titles.lang,titles.title,titles.latin,titles.official,titles.main,\
languages,platforms,released,rating,votecount,length_minutes,tags.id,tags.rating,tags.spoiler,tags.lie";

pub const VN_FULL_FIELDS: &str = "title,alttitle,olang,titles.lang,titles.title,titles.latin,titles.official,titles.main,\
languages,platforms,released,rating,votecount,length_minutes,description,tags.id,tags.rating,tags.spoiler,tags.lie,\
developers.id,developers.name,extlinks.url,extlinks.label,extlinks.name";

pub const RELEASE_FIELDS: &str = "title,released,languages.lang,languages.mtl,platforms,freeware,official,patch,\
extlinks.url,extlinks.label,extlinks.name,producers.id,producers.name,producers.developer,producers.publisher";

impl Client {
    pub fn vn_by_id(&self, id: &str) -> Result<Option<Vn>> {
        let r: Fetched<Vn> = self.query("vn", json!(["id", "=", id]), VN_FULL_FIELDS, None, 1)?;
        Ok(r.items.into_iter().next())
    }

    pub fn vn_search_title(&self, q: &str, max: usize) -> Result<Vec<Vn>> {
        let r: Fetched<Vn> = self.query(
            "vn",
            json!(["search", "=", q]),
            VN_LIST_FIELDS,
            Some(("searchrank", false)),
            max,
        )?;
        Ok(r.items)
    }

    pub fn releases_for(&self, vn_id: &str) -> Result<Vec<Release>> {
        let r: Fetched<Release> = self.query(
            "release",
            json!(["vn", "=", ["id", "=", vn_id]]),
            RELEASE_FIELDS,
            Some(("released", false)),
            300,
        )?;
        Ok(r.items)
    }

    /// Fetch developer/publisher websites.
    pub fn producer_links(&self, ids: &[String]) -> Result<Vec<(String, Vec<ExtLink>)>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        #[derive(Deserialize)]
        struct P {
            name: String,
            #[serde(default)]
            extlinks: Vec<ExtLink>,
        }
        let mut f = vec![json!("or")];
        f.extend(ids.iter().map(|id| json!(["id", "=", id])));
        let r: Fetched<P> = self.query(
            "producer",
            Value::Array(f),
            "name,extlinks.url,extlinks.label,extlinks.name",
            None,
            ids.len(),
        )?;
        Ok(r.items.into_iter().map(|p| (p.name, p.extlinks)).collect())
    }
}
