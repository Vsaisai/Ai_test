//! "Shelves": named, pre-defined searches grouped into sections (genres, 18+, gameplay ...).
//!
//! Defaults are embedded; a `shelves.json` in the data directory overrides them.

use crate::catalog::Catalog;
use crate::search::{search, SearchRequest, SearchResponse};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

pub const DEFAULT: &str = include_str!("../assets/shelves.json");

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Shelf {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub adult: bool,
    /// Partial `SearchRequest`; keys here override the user's base preferences.
    #[serde(default)]
    pub query: Value,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Section {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub adult: bool,
    pub shelves: Vec<Shelf>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Shelves {
    pub sections: Vec<Section>,
}

impl Shelves {
    pub fn load(data_dir: &Path) -> Result<Self> {
        let custom = data_dir.join("shelves.json");
        let text = if custom.exists() {
            std::fs::read_to_string(&custom)?
        } else {
            DEFAULT.to_string()
        };
        serde_json::from_str(&text)
            .with_context(|| format!("invalid shelves file {}", custom.display()))
    }

    pub fn find(&self, id: &str) -> Option<&Shelf> {
        self.sections
            .iter()
            .flat_map(|s| s.shelves.iter())
            .find(|s| s.id == id)
    }
}

/// Merge the shelf query over the base request (user preferences).
pub fn request_for(shelf: &Shelf, base: &SearchRequest) -> Result<SearchRequest> {
    let mut v = serde_json::to_value(base)?;
    if let (Some(obj), Some(q)) = (v.as_object_mut(), shelf.query.as_object()) {
        for (k, val) in q {
            obj.insert(k.clone(), val.clone());
        }
    }
    Ok(serde_json::from_value(v)?)
}

pub fn run_shelf(cat: &Catalog, shelf: &Shelf, base: &SearchRequest) -> Result<SearchResponse> {
    search(cat, &request_for(shelf, base)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_parse_and_merge() {
        let s: Shelves = serde_json::from_str(DEFAULT).unwrap();
        assert!(s.sections.len() >= 4);
        let top = s.find("top").unwrap();
        let base = SearchRequest {
            langs: vec!["ru".into()],
            ..Default::default()
        };
        let r = request_for(top, &base).unwrap();
        assert_eq!(r.min_votes, Some(300));
        assert_eq!(r.langs, vec!["ru".to_string()]);
    }
}
