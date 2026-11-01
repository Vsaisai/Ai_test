//! Human-readable terminal output.

use crate::api::Vn;
use crate::lang::{display_title, original_title, LangPref};
use crate::links::{Kind, Where};
use crate::search::{Hit, Outcome, Query};
use crate::tags::TagDb;
use std::collections::BTreeMap;

const ROOT_ORDER: &[&str] = &["Theme", "Character", "Setting", "Plot", "Style"];

fn root_rank(name: &str) -> usize {
    ROOT_ORDER.iter().position(|r| *r == name).unwrap_or(99)
}

pub fn tag_names(ids: &[u32], db: &TagDb) -> String {
    ids.iter()
        .map(|i| {
            db.get(*i)
                .map(|t| t.name.clone())
                .unwrap_or_else(|| format!("g{i}"))
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn meta_line(vn: &Vn) -> String {
    let mut parts = vec![vn.id.clone()];
    if let Some(r) = &vn.released {
        parts.push(r.chars().take(4).collect());
    }
    if let Some(r) = vn.rating {
        parts.push(format!("rating {:.1}", r / 10.0));
    }
    if let Some(m) = vn.length_minutes {
        parts.push(format!("~{}h", (m + 30) / 60));
    }
    parts.join(" · ")
}

pub fn print_search(out: &Outcome, q: &Query, db: &TagDb, lang: &LangPref, limit: usize) {
    if !q.must.is_empty() {
        println!("Required : {}", tag_names(&q.must, db));
    }
    if !q.want.is_empty() {
        println!("Wanted   : {}", tag_names(&q.want, db));
    }
    if !q.exclude.is_empty() {
        println!("Excluded : {}", tag_names(&q.exclude, db));
    }
    let total = out.total.map(|t| t.to_string()).unwrap_or("?".into());
    print!("Matches on VNDB: {total}; ranked {} ", out.hits.len());
    if out.total.map(|t| t as usize > out.fetched).unwrap_or(false) {
        print!(
            "(from the {} most-voted; raise --fetch to rank more)",
            out.fetched
        );
    }
    println!("\n");
    for (i, h) in out.hits.iter().take(limit).enumerate() {
        print_hit(i + 1, h, q, db, lang);
    }
    if out.hits.is_empty() {
        println!("Nothing found. Try fewer required tags, --any-lang, or --min-match 0.");
    }
}

fn print_hit(n: usize, h: &Hit, q: &Query, db: &TagDb, lang: &LangPref) {
    let badge = if q.want.is_empty() {
        String::new()
    } else if h.missing.is_empty() {
        format!("[ALL {}/{}] ", h.matched.len(), q.want.len())
    } else {
        format!("[{}/{}] ", h.matched.len(), q.want.len())
    };
    println!("{n:>3}. {badge}{}", h.title);
    let langs = lang.sorted_langs(&h.vn.languages);
    println!("      {}  |  langs: {}", meta_line(&h.vn), langs.join(" "));
    if !q.want.is_empty() {
        if !h.matched.is_empty() {
            println!("      has: {}", tag_names(&h.matched, db));
        }
        if !h.missing.is_empty() {
            println!("      lacks: {}", tag_names(&h.missing, db));
        }
    }
}

/// `per_group`: max entries shown per group (0 = all).
pub fn print_where(w: &Where, indent: &str, per_group: usize) {
    let mut last: Option<Kind> = None;
    let mut shown = 0;
    for (i, s) in w.sources.iter().enumerate() {
        if last != Some(s.kind) {
            println!("{indent}{}:", heading(s.kind));
            last = Some(s.kind);
            shown = 0;
        }
        shown += 1;
        if per_group > 0 && shown > per_group {
            if shown == per_group + 1 {
                let rest = w.sources[i..]
                    .iter()
                    .take_while(|x| x.kind == s.kind)
                    .count();
                println!("{indent}  ... {rest} more (use --all)");
            }
            continue;
        }
        let mut extra = Vec::new();
        if !s.langs.is_empty() {
            extra.push(s.langs.join(","));
        }
        if s.mtl {
            extra.push("machine translation".into());
        }
        if let Some(p) = &s.price {
            extra.push(format!("price {p}"));
        }
        let extra = if extra.is_empty() {
            String::new()
        } else {
            format!(" [{}]", extra.join("; "))
        };
        println!("{indent}  - {}{extra}: {}", s.site, s.url);
        if let Some(r) = &s.release {
            println!("{indent}      release: {r}");
        }
    }
    println!("{indent}VNDB page: {}", w.vndb);
}

fn heading(k: Kind) -> &'static str {
    match k {
        Kind::Free => "Free download (official)",
        Kind::FanPatch => "Fan translations / patches",
        Kind::Store => "Buy in a store",
        Kind::Direct => "Buy directly from developer/publisher",
        Kind::Search => "Not listed on VNDB - search on",
    }
}

/// Strip VNDB BBCode. Spoilers are hidden unless `spoilers` is true.
pub fn clean_description(s: &str, spoilers: bool) -> String {
    const KNOWN: &[&str] = &["b", "i", "u", "s", "url", "spoiler", "quote", "raw", "code"];
    let is_tag = |t: &str| {
        let name = t.trim_start_matches('/');
        let name = name.split('=').next().unwrap_or("");
        KNOWN.contains(&name.to_lowercase().as_str())
    };
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('[') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find(']');
        match end.map(|e| &after[..e]) {
            Some(tag) if is_tag(tag) => {
                rest = &after[end.unwrap() + 1..];
                if tag == "spoiler" && !spoilers {
                    match rest.find("[/spoiler]") {
                        Some(e) => {
                            out.push_str("[spoiler hidden]");
                            rest = &rest[e + "[/spoiler]".len()..];
                        }
                        None => rest = "",
                    }
                }
            }
            _ => {
                out.push('[');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

pub fn print_vn(vn: &Vn, db: &TagDb, lang: &LangPref, spoiler: u8, show_ero: bool) {
    let title = display_title(vn, lang);
    println!("{title}");
    if let Some(o) = original_title(vn, &title) {
        println!("  original: {o}");
    }
    println!("  {}", meta_line(vn));
    if let Some(v) = vn.votecount {
        println!("  votes: {v}");
    }
    println!(
        "  languages: {}",
        lang.sorted_langs(&vn.languages).join(" ")
    );
    if !vn.platforms.is_empty() {
        println!("  platforms: {}", vn.platforms.join(" "));
    }
    if !vn.developers.is_empty() {
        let d: Vec<_> = vn.developers.iter().map(|d| d.name.as_str()).collect();
        println!("  developers: {}", d.join(", "));
    }
    let other_titles: Vec<String> = vn
        .titles
        .iter()
        .filter(|t| t.title != title && !t.main)
        .map(|t| format!("{} ({})", t.title, t.lang))
        .collect();
    if !other_titles.is_empty() {
        println!("  also known as: {}", other_titles.join("; "));
    }
    if let Some(d) = &vn.description {
        println!("\n{}\n", clean_description(d, spoiler >= 2));
    }
    print_vn_tags(vn, db, spoiler, show_ero);
}

/// Tags grouped as  Root > Sub-category : tag (rating), ...
pub fn print_vn_tags(vn: &Vn, db: &TagDb, spoiler: u8, show_ero: bool) {
    let mut groups: BTreeMap<(usize, String, String), Vec<(f64, String)>> = BTreeMap::new();
    let mut hidden = 0;
    for t in &vn.tags {
        let Ok(id) = t.id.trim_start_matches('g').parse::<u32>() else {
            continue;
        };
        let Some(tag) = db.get(id) else { continue };
        if t.lie || t.rating <= 0.0 {
            continue;
        }
        if t.spoiler > spoiler || (!show_ero && tag.cat == "ero") {
            hidden += 1;
            continue;
        }
        let (root, sub) = db.category(id);
        let mark = match t.spoiler {
            1 => " (minor spoiler)",
            2 => " (SPOILER)",
            _ => "",
        };
        groups
            .entry((root_rank(&root), root, sub))
            .or_default()
            .push((t.rating, format!("{}{mark}", tag.name)));
    }
    println!("Tags:");
    let mut cur_root = String::new();
    for ((_, root, sub), mut tags) in groups {
        if root != cur_root {
            println!("  {root}");
            cur_root = root;
        }
        tags.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        let list: Vec<String> = tags.iter().map(|(r, n)| format!("{n} {r:.1}")).collect();
        println!("    {sub}: {}", list.join(", "));
    }
    if hidden > 0 {
        println!("  ({hidden} tags hidden: spoilers / sexual content; see --spoiler, --ero)");
    }
}

/// Category tree of all tags.
pub fn print_tree(db: &TagDb, root: Option<u32>, depth: usize, show_ero: bool) {
    let mut roots = match root {
        Some(r) => vec![r],
        None => db.roots(),
    };
    roots.sort_by_key(|r| db.get(*r).map(|t| root_rank(&t.name)).unwrap_or(99));
    for r in roots {
        walk(db, r, 0, depth, show_ero);
    }
}

fn walk(db: &TagDb, id: u32, level: usize, depth: usize, show_ero: bool) {
    let Some(t) = db.get(id) else { return };
    if !show_ero && t.cat == "ero" {
        return;
    }
    let kids = db.children(id);
    let more = if level + 1 >= depth && !kids.is_empty() {
        format!(" (+{} sub-tags)", kids.len())
    } else {
        String::new()
    };
    let count = if t.vns > 0 {
        format!("  [{} VNs]", t.vns)
    } else {
        String::new()
    };
    println!("{}g{:<5} {}{count}{more}", "  ".repeat(level), t.id, t.name);
    if level + 1 < depth {
        let mut kids = kids.to_vec();
        kids.sort_by_key(|k| db.get(*k).map(|t| t.name.to_lowercase()));
        for k in kids {
            walk(db, k, level + 1, depth, show_ero);
        }
    }
}

pub fn print_tag_matches(db: &TagDb, q: &str, show_ero: bool) {
    let found: Vec<u32> = db
        .search(q)
        .into_iter()
        .filter(|i| show_ero || db.get(*i).map(|t| t.cat != "ero").unwrap_or(true))
        .collect();
    if found.is_empty() {
        println!("No tags match \"{q}\".");
        return;
    }
    for id in found.iter().take(40) {
        let t = db.get(*id).unwrap();
        let flags = match (t.searchable, t.applicable) {
            (false, _) => " (category only, not searchable)",
            (true, false) => " (group: matches its sub-tags)",
            _ => "",
        };
        println!(
            "g{:<5} {}  [{} VNs]{flags}",
            t.id,
            db.path_names(*id),
            t.vns
        );
        if !t.aliases.is_empty() {
            println!("       aliases: {}", t.aliases.join(", "));
        }
    }
    if found.len() > 40 {
        println!("... {} more, refine the query", found.len() - 40);
    }
}

pub fn print_tag_info(db: &TagDb, id: u32) {
    let t = db.get(id).unwrap();
    println!("g{}  {}", t.id, db.path_names(id));
    println!("  category: {}  |  {} VNs", t.cat, t.vns);
    if !t.aliases.is_empty() {
        println!("  aliases: {}", t.aliases.join(", "));
    }
    if !t.description.is_empty() {
        println!("\n{}\n", clean_description(&t.description, true));
    }
    let kids = db.children(id);
    if !kids.is_empty() {
        println!("Sub-tags (also matched when searching for this tag):");
        print_tree_children(db, id);
    }
}

fn print_tree_children(db: &TagDb, id: u32) {
    let mut kids = db.children(id).to_vec();
    kids.sort_by_key(|k| db.get(*k).map(|t| t.name.to_lowercase()));
    for k in kids {
        walk(db, k, 1, 2, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bbcode_is_stripped() {
        let d = "Hello [url=https://x]world[/url]. [spoiler]secret[/spoiler] end";
        assert_eq!(
            clean_description(d, false),
            "Hello world. [spoiler hidden] end"
        );
        assert_eq!(clean_description(d, true), "Hello world. secret end");
        assert_eq!(
            clean_description("[From [url=https://w]Wikipedia[/url]]", false),
            "[From Wikipedia]"
        );
    }
}
