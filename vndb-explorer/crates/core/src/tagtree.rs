//! Tag hierarchy with categorization (Root > Sub-category) and name resolution.

use anyhow::{bail, Result};
use rusqlite::Connection;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct Tag {
    pub id: u32,
    pub name: String,
    /// "cont", "ero" (sexual content) or "tech"
    pub cat: String,
    pub aliases: Vec<String>,
    pub searchable: bool,
    pub applicable: bool,
    /// Main parent first.
    pub parents: Vec<u32>,
    pub children: Vec<u32>,
    /// Number of VNs having this tag or one of its sub-tags.
    pub vns: u32,
}

pub struct TagTree {
    pub tags: HashMap<u32, Tag>,
    pub max_id: u32,
}

/// Bit set over tag ids.
#[derive(Clone)]
pub struct TagSet(Vec<u64>);

impl TagSet {
    pub fn new(max: u32) -> Self {
        Self(vec![0; max as usize / 64 + 1])
    }
    pub fn insert(&mut self, id: u32) {
        if let Some(w) = self.0.get_mut(id as usize / 64) {
            *w |= 1 << (id % 64);
        }
    }
    pub fn contains(&self, id: u32) -> bool {
        self.0
            .get(id as usize / 64)
            .map(|w| w & (1 << (id % 64)) != 0)
            .unwrap_or(false)
    }
}

pub const ROOT_ORDER: &[&str] = &["Theme", "Character", "Setting", "Plot", "Style"];

impl TagTree {
    pub fn load(c: &Connection) -> Result<Self> {
        let mut tags = HashMap::new();
        let mut st =
            c.prepare("SELECT id, name, cat, aliases, searchable, applicable FROM tags")?;
        let rows = st.query_map([], |r| {
            let aliases: String = r.get(3)?;
            Ok(Tag {
                id: r.get(0)?,
                name: r.get(1)?,
                cat: r.get(2)?,
                aliases: aliases
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .collect(),
                searchable: r.get(4)?,
                applicable: r.get(5)?,
                parents: vec![],
                children: vec![],
                vns: 0,
            })
        })?;
        for t in rows {
            let t = t?;
            tags.insert(t.id, t);
        }
        let mut st = c.prepare("SELECT id, parent, main FROM tag_parents ORDER BY main DESC")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, u32>(0)?, r.get::<_, u32>(1)?)))?;
        for row in rows {
            let (id, parent) = row?;
            if let Some(t) = tags.get_mut(&id) {
                t.parents.push(parent);
            }
            if let Some(p) = tags.get_mut(&parent) {
                p.children.push(id);
            }
        }
        let max_id = tags.keys().copied().max().unwrap_or(0);
        let mut tree = Self { tags, max_id };
        let mut kids: Vec<(u32, Vec<u32>)> = tree
            .tags
            .iter()
            .map(|(id, t)| (*id, t.children.clone()))
            .collect();
        for (_, k) in kids.iter_mut() {
            k.sort_by_key(|i| tree.tags.get(i).map(|t| t.name.to_lowercase()));
        }
        for (id, k) in kids {
            tree.tags.get_mut(&id).unwrap().children = k;
        }
        Ok(tree)
    }

    pub fn get(&self, id: u32) -> Option<&Tag> {
        self.tags.get(&id)
    }

    /// Main-parent chain from root to `id`.
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

    pub fn path_names(&self, id: u32) -> Vec<String> {
        self.path(id)
            .iter()
            .filter_map(|i| self.get(*i).map(|t| t.name.clone()))
            .collect()
    }

    /// (root, sub-category) names.
    pub fn category(&self, id: u32) -> (String, String) {
        let p = self.path_names(id);
        let root = p.first().cloned().unwrap_or_default();
        let sub = if p.len() > 2 {
            p[1].clone()
        } else {
            "General".into()
        };
        (root, sub)
    }

    /// Tag and all its descendants (through every parent link).
    pub fn subtree(&self, id: u32) -> TagSet {
        let mut set = TagSet::new(self.max_id);
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if set.contains(cur) {
                continue;
            }
            set.insert(cur);
            if let Some(t) = self.get(cur) {
                stack.extend(t.children.iter().copied());
            }
        }
        set
    }

    /// Resolve "g123", "123", an exact name/alias, a unique prefix or unique substring.
    pub fn resolve(&self, input: &str) -> Result<u32> {
        let q = input.trim();
        let num = q.strip_prefix(['g', 'G']).unwrap_or(q);
        if let Ok(id) = num.parse::<u32>() {
            if self.tags.contains_key(&id) {
                return Ok(id);
            }
        }
        let ql = q.to_lowercase();
        let mut exact: Vec<&Tag> = self
            .tags
            .values()
            .filter(|t| t.name.to_lowercase() == ql)
            .collect();
        if exact.is_empty() {
            exact = self
                .tags
                .values()
                .filter(|t| t.aliases.iter().any(|a| a.to_lowercase() == ql))
                .collect();
        }
        if exact.len() == 1 {
            return Ok(exact[0].id);
        }
        let found = self.search(q, 50);
        let prefixed: Vec<u32> = found
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
        match found.len() {
            0 => bail!("unknown tag \"{q}\""),
            1 => Ok(found[0]),
            _ => bail!("tag \"{q}\" is ambiguous"),
        }
    }

    /// Name/alias substring search, most used first.
    pub fn search(&self, q: &str, limit: usize) -> Vec<u32> {
        let ql = q.to_lowercase();
        let mut v: Vec<&Tag> = self
            .tags
            .values()
            .filter(|t| {
                t.name.to_lowercase().contains(&ql)
                    || t.aliases.iter().any(|a| a.to_lowercase().contains(&ql))
            })
            .collect();
        v.sort_by(|a, b| {
            let ea = a.name.to_lowercase() == ql;
            let eb = b.name.to_lowercase() == ql;
            eb.cmp(&ea)
                .then(b.vns.cmp(&a.vns))
                .then(a.name.cmp(&b.name))
        });
        v.into_iter().take(limit).map(|t| t.id).collect()
    }

    pub fn roots(&self) -> Vec<u32> {
        let mut r: Vec<u32> = self
            .tags
            .values()
            .filter(|t| t.parents.is_empty())
            .map(|t| t.id)
            .collect();
        r.sort_by_key(|id| {
            let n = &self.tags[id].name;
            ROOT_ORDER.iter().position(|x| x == n).unwrap_or(99)
        });
        r
    }

    /// Is the tag (or any ancestor) in the sexual-content category?
    pub fn is_ero(&self, id: u32) -> bool {
        self.get(id).map(|t| t.cat == "ero").unwrap_or(false)
    }
}
