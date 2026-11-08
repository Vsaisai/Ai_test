//! `vndb-explorer`: the VN Shelf catalog in a terminal.
//!
//! A thin front-end over `vndb-core`: the same local database, tag tree, ranked tag search and
//! "where to get" links as the app (`vndb-web import` / the app's "Download database" fill it).

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand};
use std::path::PathBuf;
use vndb_core::detail::{self, Detail, SourceKind};
use vndb_core::search::{EroMode, LangMode, Sort};
use vndb_core::{search, Catalog, SearchRequest};

/// Browse the local VNDB catalog: categorized tags, strict/preferred tag search, readable
/// (English or other non-CJK) titles and where to get games.
#[derive(Parser)]
#[command(name = "vndb-explorer", version)]
struct Cli {
    /// Data directory with catalog.sqlite (the same one the app / vndb-web uses)
    #[arg(long, global = true, env = "VNDB_DATA")]
    data: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Args, Clone)]
struct LangArgs {
    /// Preferred languages, comma separated, in priority order (e.g. "en,ru").
    /// Only VNs available in one of them are shown. Default: any non-CJK language
    #[arg(short, long)]
    lang: Option<String>,
    /// Do not filter by language at all (include Japanese/Chinese/Korean-only VNs)
    #[arg(long)]
    any_lang: bool,
}

impl LangArgs {
    fn order(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .lang
            .as_deref()
            .unwrap_or("en")
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if v.is_empty() {
            v.push("en".into());
        }
        v
    }
    fn mode(&self) -> LangMode {
        if self.any_lang {
            LangMode::Any
        } else if self.lang.is_some() {
            LangMode::List
        } else {
            LangMode::Readable
        }
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Browse the categorized tag tree, or look a tag up by name
    Tags {
        /// Name / alias / id (g123). Omit to print the category tree
        query: Option<String>,
        /// Tree depth (with no query)
        #[arg(short, long, default_value_t = 2)]
        depth: usize,
        /// Show sexual-content tags too
        #[arg(long)]
        ero: bool,
    },
    /// Find VNs by tags: required (-m), wanted (-w), excluded (-x)
    Search {
        /// Tag that MUST be present (repeatable). Sub-tags count too
        #[arg(short = 'm', long = "must")]
        must: Vec<String>,
        /// Tag that is WANTED (repeatable): VNs with all wanted tags go first,
        /// then by number of matched tags, then alphabetically
        #[arg(short = 'w', long = "want")]
        want: Vec<String>,
        /// Tag that must NOT be present (repeatable)
        #[arg(short = 'x', long = "exclude")]
        exclude: Vec<String>,
        /// Also filter by title text
        #[arg(short, long)]
        title: Option<String>,
        /// Minimum number of wanted tags a result needs (0 = wanted tags only rank)
        #[arg(long, default_value_t = 0)]
        min_match: usize,
        /// Spoiler level of tags to consider: 0 none, 1 minor, 2 all
        #[arg(long, default_value_t = 0)]
        spoiler: u8,
        /// How many results to print
        #[arg(short = 'n', long, default_value_t = 25)]
        limit: usize,
        /// Also show where to get each printed result
        #[arg(long)]
        links: bool,
        #[command(flatten)]
        lang: LangArgs,
        /// Machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Full info about one VN: description, categorized tags and where to get it
    Vn {
        /// VNDB id (v17 / 17) or a title
        query: String,
        /// Spoiler level of tags/description: 0 none, 1 minor, 2 all
        #[arg(long, default_value_t = 0)]
        spoiler: u8,
        /// Show sexual-content tags
        #[arg(long)]
        ero: bool,
        #[command(flatten)]
        lang: LangArgs,
        #[arg(long)]
        json: bool,
    },
    /// Only the "where to download / buy" part for a VN
    Links {
        /// VNDB id or title
        query: String,
        #[command(flatten)]
        lang: LangArgs,
        #[arg(long)]
        json: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data = cli.data.unwrap_or_else(vndb_core::db::default_data_dir);
    let db = data.join("catalog.sqlite");
    if !db.exists() {
        bail!(
            "no local database in {}: run `vndb-web import` (or press \"Download database\" in the app) first, or pass --data",
            data.display()
        );
    }
    let cat = Catalog::open(&db).with_context(|| format!("cannot open {}", db.display()))?;
    match cli.cmd {
        Cmd::Tags { query, depth, ero } => {
            match query {
                None => {
                    for root in cat.tree.roots() {
                        print_tree(&cat, root, 0, depth, ero);
                    }
                }
                Some(q) => print_tag(&cat, &q)?,
            }
            Ok(())
        }
        Cmd::Search {
            must,
            want,
            exclude,
            title,
            min_match,
            spoiler,
            limit,
            links,
            lang,
            json,
        } => {
            let req = SearchRequest {
                text: title,
                must,
                want,
                exclude,
                min_match: Some(min_match),
                spoiler,
                lang_mode: lang.mode(),
                langs: lang.order(),
                sort: Sort::Relevance,
                limit,
                ..SearchRequest::default()
            };
            let res = search(&cat, &req)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&res)?);
                return Ok(());
            }
            for w in &res.warnings {
                eprintln!("warning: {w}");
            }
            let wanted = res.want.len();
            println!("{} found", res.total);
            for (n, c) in res.items.iter().enumerate() {
                let score = if wanted > 0 {
                    format!(" [{}/{wanted}]", c.matched.len())
                } else {
                    String::new()
                };
                println!("{:>3}. {}{score}  {}", n + 1, c.title, meta(c));
                if links {
                    if let Some(d) = detail_of(&cat, num(&c.id), &lang)? {
                        print_sources(&d, "      ");
                    }
                }
            }
            Ok(())
        }
        Cmd::Vn {
            query,
            spoiler,
            ero,
            lang,
            json,
        } => {
            let d = find_vn(&cat, &query, &lang)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
                return Ok(());
            }
            print_vn(&d, spoiler, ero);
            Ok(())
        }
        Cmd::Links { query, lang, json } => {
            let d = find_vn(&cat, &query, &lang)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d.sources)?);
                return Ok(());
            }
            println!("{}", d.card.title);
            print_sources(&d, "  ");
            Ok(())
        }
    }
}

fn num(id: &str) -> u32 {
    id.trim_start_matches(['v', 'V']).parse().unwrap_or(0)
}

fn detail_of(cat: &Catalog, id: u32, lang: &LangArgs) -> Result<Option<Detail>> {
    detail::detail(cat, id, &lang.order(), lang.any_lang)
}

/// `v17`, `17` or a title (the best match of a title search).
fn find_vn(cat: &Catalog, q: &str, lang: &LangArgs) -> Result<Detail> {
    let digits = q.trim().trim_start_matches(['v', 'V']);
    let id = if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        digits.parse()?
    } else {
        let req = SearchRequest {
            text: Some(q.to_string()),
            lang_mode: LangMode::Any,
            ero: EroMode::Show,
            sort: Sort::Votes,
            limit: 1,
            ..SearchRequest::default()
        };
        match search(cat, &req)?.items.first() {
            Some(c) => num(&c.id),
            None => bail!("no VN matches \"{q}\""),
        }
    };
    detail_of(cat, id, lang)?.with_context(|| format!("no VN v{id} in the local database"))
}

fn meta(c: &vndb_core::search::Card) -> String {
    let mut parts = Vec::new();
    if let Some(r) = &c.released {
        parts.push(r.clone());
    }
    if let Some(r) = c.rating {
        parts.push(format!("★{r:.1} ({} votes)", c.votes));
    }
    if !c.langs.is_empty() {
        parts.push(
            c.langs
                .iter()
                .map(|l| l.lang.as_str())
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    format!("{} · {}", c.id, parts.join(" · "))
}

fn print_tree(cat: &Catalog, id: u32, level: usize, depth: usize, ero: bool) {
    let Some(t) = cat.tree.get(id) else { return };
    if !ero && cat.tree.is_ero(id) {
        return;
    }
    println!(
        "{}{} (g{}, {} VNs)",
        "  ".repeat(level),
        t.name,
        t.id,
        t.vns
    );
    if level + 1 < depth {
        for &k in &t.children {
            print_tree(cat, k, level + 1, depth, ero);
        }
    }
}

fn print_tag(cat: &Catalog, q: &str) -> Result<()> {
    let id = match cat.tree.resolve(q) {
        Ok(id) => id,
        Err(e) => {
            let similar = cat.tree.search(q, 10);
            if similar.is_empty() {
                return Err(e);
            }
            println!("{e}. Similar tags:");
            for id in similar {
                if let Some(t) = cat.tree.get(id) {
                    println!("  g{:<6} {}  ({} VNs)", t.id, t.name, t.vns);
                }
            }
            return Ok(());
        }
    };
    let t = cat.tree.get(id).context("tag")?;
    println!("{} (g{})", t.name, t.id);
    println!("{}", cat.tree.path_names(id).join(" > "));
    if !t.aliases.is_empty() {
        println!("Also: {}", t.aliases.join(", "));
    }
    println!("{} VNs", t.vns);
    let raw: String = cat
        .connection()?
        .query_row("SELECT description FROM tags WHERE id = ?1", [id], |r| {
            r.get(0)
        })
        .unwrap_or_default();
    let d = detail::parse_description(&raw);
    if !d.text.is_empty() {
        println!("\n{}", d.text);
    }
    if !d.examples.is_empty() {
        println!(
            "Examples: {} screenshot(s) on https://vndb.org/g{id}",
            d.examples.len()
        );
    }
    let kids: Vec<_> = t.children.iter().filter_map(|k| cat.tree.get(*k)).collect();
    if !kids.is_empty() {
        println!("\nSub-tags:");
        for k in kids {
            println!("  g{:<6} {}  ({} VNs)", k.id, k.name, k.vns);
        }
    }
    Ok(())
}

fn print_vn(d: &Detail, spoiler: u8, ero: bool) {
    println!("{}", d.card.title);
    if let Some(o) = &d.card.original {
        println!("{o}");
    }
    println!("{}  {}", meta(&d.card), d.vndb_url);
    if !d.developers.is_empty() {
        println!("By: {}", d.developers.join(", "));
    }
    let desc = hide_spoilers(&d.description, spoiler);
    if !desc.is_empty() {
        println!("\n{desc}");
    }
    for cat in &d.tags {
        let mut lines = Vec::new();
        for g in &cat.groups {
            let names: Vec<&str> = g
                .tags
                .iter()
                .filter(|t| t.spoiler <= spoiler && (ero || !t.ero))
                .map(|t| t.name.as_str())
                .collect();
            if !names.is_empty() {
                lines.push(format!("  {}: {}", g.name, names.join(", ")));
            }
        }
        if !lines.is_empty() {
            println!("\n{}", cat.name);
            println!("{}", lines.join("\n"));
        }
    }
    println!();
    print_sources(d, "  ");
}

/// `[spoiler]…[/spoiler]` blocks are cut unless all spoilers are allowed.
fn hide_spoilers(s: &str, level: u8) -> String {
    if level >= 2 {
        return s.replace("[spoiler]", "").replace("[/spoiler]", "");
    }
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("[spoiler]") {
        out.push_str(&rest[..i]);
        match rest[i..].find("[/spoiler]") {
            Some(j) => {
                out.push_str("[spoiler]");
                rest = &rest[i + j + "[/spoiler]".len()..];
            }
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out.trim().to_string()
}

fn print_sources(d: &Detail, indent: &str) {
    if d.sources.is_empty() {
        println!("{indent}Where to get: nothing known");
        return;
    }
    let mut last = None;
    for s in &d.sources {
        if last != Some(s.kind) {
            println!("{indent}{}:", heading(s.kind));
            last = Some(s.kind);
        }
        let mut extra = Vec::new();
        if !s.langs.is_empty() {
            extra.push(s.langs.join(","));
        }
        if s.mtl {
            extra.push("machine translation".into());
        }
        let extra = if extra.is_empty() {
            String::new()
        } else {
            format!(" ({})", extra.join(", "))
        };
        println!("{indent}  {}{extra}: {}", s.site, s.url);
    }
}

fn heading(k: SourceKind) -> &'static str {
    match k {
        SourceKind::Free => "Free download",
        SourceKind::Store => "Stores",
        SourceKind::Direct => "From the developer",
        SourceKind::FanPatch => "Fan translations",
        SourceKind::Original => "Original release",
        SourceKind::Search => "Search",
    }
}

#[cfg(test)]
mod tests {
    use super::hide_spoilers;

    #[test]
    fn spoilers_are_cut_below_level_two() {
        assert_eq!(
            hide_spoilers("a [spoiler]b[/spoiler] c", 0),
            "a [spoiler] c"
        );
        assert_eq!(hide_spoilers("a [spoiler]b[/spoiler] c", 2), "a b c");
    }
}
