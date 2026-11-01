//! VNDB tag tree: download/cache the public tag dump, resolve names, categorize.
//!
//! The Kana API does not expose tag parents, so the daily database dump
//! (`vndb-tags-latest.json.gz`) is used to rebuild the hierarchy. Every tag is
//! then filed under `Root category > Sub-category`, e.g. `Theme > Drama`.

use anyhow::{bail, Context, Result};
use flate2::read::GzDecoder;
use serde::Deserialize;
use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

const DUMP_URL: &str = "https://dl.vndb.org/dump/vndb-tags-latest.json.gz";
const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);

#[derive(Debug, Clone, Deserialize)]
pub struct Tag {
    pub id: u32,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub description: String,
    /// "cont" (content), "ero" (sexual content), "tech" (technical)
    pub cat: String,
    #[serde(default)]
    pub parents: Vec<u32>,
    #[serde(default)]
    pub searchable: bool,
    #[serde(default)]
    pub applicable: bool,
    #[serde(default)]
    pub vns: u64,
}

pub struct TagDb {
    pub tags: HashMap<u32, Tag>,
    children: HashMap<u32, Vec<u32>>,
}

fn cache_file() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("vndb-explorer")
        .join("vndb-tags.json.gz")
}

impl TagDb {
    /// Load from cache, downloading the dump if missing, stale or `refresh` is set.
    pub fn load(http: &reqwest::blocking::Client, refresh: bool) -> Result<Self> {
        let path = cache_file();
        let fresh = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .map(|age| age < MAX_AGE)
            .unwrap_or(false);
        if refresh || !fresh {
            eprintln!("Downloading VNDB tag tree ...");
            match download(http) {
                Ok(bytes) => {
                    if let Some(dir) = path.parent() {
                        std::fs::create_dir_all(dir)?;
                    }
                    std::fs::write(&path, &bytes)?;
                }
                Err(e) if path.exists() => eprintln!("warning: {e:#}; using cached tag tree"),
                Err(e) => return Err(e),
            }
        }
        let raw = std::fs::read(&path)?;
        Self::from_gz(&raw)
    }

    pub fn from_gz(raw: &[u8]) -> Result<Self> {
        let mut s = String::new();
        GzDecoder::new(raw)
            .read_to_string(&mut s)
            .context("tag dump is not valid gzip")?;
        let list: Vec<Tag> = serde_json::from_str(&s).context("tag dump is not valid JSON")?;
        Ok(Self::from_list(list))
    }

    pub fn from_list(list: Vec<Tag>) -> Self {
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for t in &list {
            for p in &t.parents {
                children.entry(*p).or_default().push(t.id);
            }
        }
        for v in children.values_mut() {
            v.sort();
        }
        let tags = list.into_iter().map(|t| (t.id, t)).collect();
        Self { tags, children }
    }

    pub fn get(&self, id: u32) -> Option<&Tag> {
        self.tags.get(&id)
    }

    pub fn children(&self, id: u32) -> &[u32] {
        self.children.get(&id).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn roots(&self) -> Vec<u32> {
        let mut r: Vec<u32> = self
            .tags
            .values()
            .filter(|t| t.parents.is_empty())
            .map(|t| t.id)
            .collect();
        r.sort();
        r
    }

    /// Primary path from the root down to `id` (follows the first parent).
    pub fn path(&self, id: u32) -> Vec<u32> {
        let mut out = vec![id];
        let mut cur = id;
        while let Some(p) = self.get(cur).and_then(|t| t.parents.first()).copied() {
            if out.contains(&p) {
                break;
            }
            out.push(p);
            cur = p;
        }
        out.reverse();
        out
    }

    pub fn path_names(&self, id: u32) -> String {
        self.path(id)
            .iter()
            .filter_map(|i| self.get(*i).map(|t| t.name.as_str()))
            .collect::<Vec<_>>()
            .join(" > ")
    }

    /// (root name, sub-category name) used for grouping.
    pub fn category(&self, id: u32) -> (String, String) {
        let p = self.path(id);
        let name = |i: Option<&u32>| {
            i.and_then(|i| self.get(*i))
                .map(|t| t.name.clone())
                .unwrap_or_default()
        };
        let root = name(p.first());
        let sub = if p.len() > 2 {
            name(p.get(1))
        } else {
            "General".into()
        };
        (root, sub)
    }

    /// True if `id` equals `ancestor` or lies anywhere beneath it (any parent chain).
    pub fn is_descendant(&self, id: u32, ancestor: u32) -> bool {
        let mut stack = vec![id];
        let mut seen = std::collections::HashSet::new();
        while let Some(cur) = stack.pop() {
            if cur == ancestor {
                return true;
            }
            if !seen.insert(cur) {
                continue;
            }
            if let Some(t) = self.get(cur) {
                stack.extend(t.parents.iter().copied());
            }
        }
        false
    }

    /// Resolve user input ("g123", "123", exact name/alias, or unique substring).
    pub fn resolve(&self, input: &str) -> Result<u32> {
        let q = input.trim();
        let num = q.strip_prefix('g').or(q.strip_prefix('G')).unwrap_or(q);
        if let Ok(id) = num.parse::<u32>() {
            if self.tags.contains_key(&id) {
                return Ok(id);
            }
        }
        let ql = q.to_lowercase();
        let exact: Vec<u32> = self
            .tags
            .values()
            .filter(|t| {
                t.name.to_lowercase() == ql || t.aliases.iter().any(|a| a.to_lowercase() == ql)
            })
            .map(|t| t.id)
            .collect();
        if exact.len() == 1 {
            return Ok(exact[0]);
        }
        let mut partial = self.search(q);
        if exact.len() > 1 {
            partial.retain(|i| exact.contains(i));
        }
        // Prefer a single tag whose name starts with the query ("Detective" -> "Detective Work").
        let prefixed: Vec<u32> = partial
            .iter()
            .copied()
            .filter(|i| {
                self.get(*i)
                    .map(|t| t.name.to_lowercase().starts_with(&ql))
                    .unwrap_or(false)
            })
            .collect();
        if prefixed.len() == 1 {
            return Ok(prefixed[0]);
        }
        match partial.len() {
            0 => bail!("no tag matches \"{q}\" (try `vndb-explorer tags {q}`)"),
            1 => Ok(partial[0]),
            _ => {
                let list: Vec<String> = partial
                    .iter()
                    .take(10)
                    .map(|i| format!("  g{i}  {}", self.path_names(*i)))
                    .collect();
                bail!(
                    "\"{q}\" is ambiguous, use the tag id or the exact name:\n{}",
                    list.join("\n")
                )
            }
        }
    }

    /// Tags whose name or alias contains the query (case-insensitive), most used first.
    pub fn search(&self, q: &str) -> Vec<u32> {
        let ql = q.to_lowercase();
        let mut v: Vec<&Tag> = self
            .tags
            .values()
            .filter(|t| {
                t.name.to_lowercase().contains(&ql)
                    || t.aliases.iter().any(|a| a.to_lowercase().contains(&ql))
            })
            .collect();
        v.sort_by(|a, b| b.vns.cmp(&a.vns).then(a.name.cmp(&b.name)));
        v.into_iter().map(|t| t.id).collect()
    }
}

fn download(http: &reqwest::blocking::Client) -> Result<Vec<u8>> {
    let resp = http
        .get(DUMP_URL)
        .send()
        .context("could not download VNDB tag dump")?;
    if !resp.status().is_success() {
        bail!("tag dump download failed: HTTP {}", resp.status());
    }
    Ok(resp.bytes()?.to_vec())
}

#[cfg(test)]
pub mod tests {
    use super::*;

    pub fn sample() -> TagDb {
        let t = |id, name: &str, parents: Vec<u32>| Tag {
            id,
            name: name.into(),
            aliases: vec![],
            description: String::new(),
            cat: "cont".into(),
            parents,
            searchable: true,
            applicable: true,
            vns: 100 - id as u64,
        };
        TagDb::from_list(vec![
            t(1, "Theme", vec![]),
            t(2, "Drama", vec![1]),
            t(3, "Tragedy", vec![2]),
            t(4, "Comedy", vec![1]),
            t(5, "Style", vec![]),
            t(6, "ADV", vec![5]),
        ])
    }

    #[test]
    fn categories_and_paths() {
        let db = sample();
        assert_eq!(db.path_names(3), "Theme > Drama > Tragedy");
        assert_eq!(db.category(3), ("Theme".into(), "Drama".into()));
        assert_eq!(db.category(4), ("Theme".into(), "General".into()));
        assert!(db.is_descendant(3, 1));
        assert!(!db.is_descendant(4, 2));
    }

    #[test]
    fn resolve_names() {
        let db = sample();
        assert_eq!(db.resolve("g3").unwrap(), 3);
        assert_eq!(db.resolve("tragedy").unwrap(), 3);
        assert_eq!(db.resolve("Trag").unwrap(), 3);
        assert!(db.resolve("nonexistent").is_err());
    }
}
