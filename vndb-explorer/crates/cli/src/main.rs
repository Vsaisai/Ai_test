mod api;
mod lang;
mod links;
mod render;
mod search;
mod tags;

use anyhow::{bail, Result};
use clap::{Args, Parser, Subcommand};
use lang::LangPref;
use serde_json::json;

/// Browse VNDB in a friendlier way: categorized tags, strict/preferred tag
/// search, readable (English or other non-CJK) titles and where to get games.
#[derive(Parser)]
#[command(name = "vndb-explorer", version)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Re-download the tag tree even if the cache is fresh
    #[arg(long, global = true)]
    refresh_tags: bool,
}

#[derive(Args, Clone)]
struct LangArgs {
    /// Preferred languages, comma separated, in priority order (e.g. "en,ru").
    /// Only VNs available in one of them are shown.
    /// Default: English first, but any non-CJK language is accepted.
    #[arg(short, long)]
    lang: Option<String>,
    /// Do not filter by language at all (include Japanese/Chinese/Korean-only VNs)
    #[arg(long)]
    any_lang: bool,
}

impl LangArgs {
    fn pref(&self) -> LangPref {
        LangPref::new(self.lang.as_deref(), self.any_lang)
    }
}

#[derive(Subcommand)]
enum Cmd {
    /// Browse the categorized tag tree, or look tags up by name
    Tags {
        /// Name / alias / part of a name / id (g123). Omit to print the category tree
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
        /// Tag that is WANTED (repeatable). VNs with all wanted tags go first,
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
        #[arg(long, default_value_t = 1)]
        min_match: usize,
        /// Spoiler level of tags to consider: 0 none, 1 minor, 2 all
        #[arg(long, default_value_t = 0)]
        spoiler: u8,
        /// Minimum tag strength (0-3) for a tag to count
        #[arg(long, default_value_t = 0.0)]
        min_level: f64,
        /// How many results to print
        #[arg(short = 'n', long, default_value_t = 25)]
        limit: usize,
        /// How many candidates to fetch from VNDB before ranking
        #[arg(long, default_value_t = 400)]
        fetch: usize,
        /// Also show download / store links for each printed result
        #[arg(long)]
        links: bool,
        /// Show every link instead of the first few per group
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        lang: LangArgs,
        /// Machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Full info about one VN: description, categorized tags and where to get it
    Vn {
        /// VNDB id (v17 / 17) or a title to search for
        query: String,
        /// Spoiler level of tags/description: 0 none, 1 minor, 2 all
        #[arg(long, default_value_t = 0)]
        spoiler: u8,
        /// Show sexual-content tags
        #[arg(long)]
        ero: bool,
        /// Look up current Steam prices for this country code (e.g. us, de, ru)
        #[arg(long, value_name = "CC")]
        prices: Option<String>,
        /// Show every link instead of the first few per group
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        lang: LangArgs,
        #[arg(long)]
        json: bool,
    },
    /// Only the "where to download / buy" part for a VN
    Links {
        /// VNDB id or title
        query: String,
        /// Look up current Steam prices for this country code
        #[arg(long, value_name = "CC")]
        prices: Option<String>,
        /// Show every link instead of the first few per group
        #[arg(long)]
        all: bool,
        #[command(flatten)]
        lang: LangArgs,
        #[arg(long)]
        json: bool,
    },
}

fn resolve_vn(client: &api::Client, q: &str, pref: &LangPref) -> Result<api::Vn> {
    let digits = q.trim().trim_start_matches(['v', 'V']);
    if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
        let id = format!("v{digits}");
        return client
            .vn_by_id(&id)?
            .ok_or_else(|| anyhow::anyhow!("{id} not found"));
    }
    let found = client.vn_search_title(q, 8)?;
    let Some(first) = found.first() else {
        bail!("no VN titled \"{q}\"")
    };
    if found.len() > 1 {
        eprintln!("Showing best match. Other matches:");
        for v in found.iter().skip(1) {
            eprintln!("  {}  {}", v.id, lang::display_title(v, pref));
        }
        eprintln!();
    }
    client
        .vn_by_id(&first.id)?
        .ok_or_else(|| anyhow::anyhow!("{} not found", first.id))
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let client = api::Client::new()?;

    match cli.cmd {
        Cmd::Tags { query, depth, ero } => {
            let db = tags::TagDb::load(client.http(), cli.refresh_tags)?;
            match query {
                None => render::print_tree(&db, None, depth.max(1), ero),
                Some(q) => match db.resolve(&q) {
                    Ok(id)
                        if q.to_lowercase() == db.get(id).unwrap().name.to_lowercase()
                            || q.starts_with(['g', 'G']) && q[1..].parse::<u32>().is_ok() =>
                    {
                        render::print_tag_info(&db, id)
                    }
                    _ => render::print_tag_matches(&db, &q, ero),
                },
            }
        }
        Cmd::Search {
            must,
            want,
            exclude,
            title,
            min_match,
            spoiler,
            min_level,
            limit,
            fetch,
            links,
            all,
            lang,
            json,
        } => {
            let db = tags::TagDb::load(client.http(), cli.refresh_tags)?;
            let ids = |v: &[String]| {
                v.iter()
                    .map(|t| db.resolve(t))
                    .collect::<Result<Vec<u32>>>()
            };
            let q = search::Query {
                must: ids(&must)?,
                want: ids(&want)?,
                exclude: ids(&exclude)?,
                min_match,
                spoiler: spoiler.min(2),
                min_level,
                title,
            };
            if q.must.is_empty() && q.want.is_empty() && q.title.is_none() {
                bail!("give at least one --must / --want tag or --title");
            }
            let pref = lang.pref();
            let out = search::run(&client, &db, &q, &pref, fetch.max(limit))?;
            if json {
                let mut items = Vec::new();
                for h in out.hits.iter().take(limit) {
                    let w = if links {
                        Some(links::lookup(&client, &h.vn, &h.title, &pref, None)?)
                    } else {
                        None
                    };
                    items.push(json!({
                        "id": h.vn.id, "title": h.title, "all_wanted": h.missing.is_empty(),
                        "matched": render::tag_names(&h.matched, &db), "missing": render::tag_names(&h.missing, &db),
                        "languages": h.vn.languages, "released": h.vn.released, "rating": h.vn.rating,
                        "url": format!("https://vndb.org/{}", h.vn.id), "where": w,
                    }));
                }
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"total": out.total, "results": items}))?
                );
            } else {
                render::print_search(&out, &q, &db, &pref, limit);
                if links {
                    println!("\n=== Where to get them ===");
                    for (i, h) in out.hits.iter().take(limit).enumerate() {
                        println!("\n{:>3}. {}", i + 1, h.title);
                        let w = links::lookup(&client, &h.vn, &h.title, &pref, None)?;
                        render::print_where(&w, "     ", if all { 0 } else { 3 });
                    }
                }
            }
        }
        Cmd::Vn {
            query,
            spoiler,
            ero,
            prices,
            all,
            lang,
            json,
        } => {
            let pref = lang.pref();
            let vn = resolve_vn(&client, &query, &pref)?;
            let title = lang::display_title(&vn, &pref);
            let w = links::lookup(&client, &vn, &title, &pref, prices.as_deref())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"title": title, "vn": vn, "where": w}))?
                );
            } else {
                let db = tags::TagDb::load(client.http(), cli.refresh_tags)?;
                render::print_vn(&vn, &db, &pref, spoiler.min(2), ero);
                println!("\nWhere to get it:");
                render::print_where(&w, "  ", if all { 0 } else { 6 });
            }
        }
        Cmd::Links {
            query,
            prices,
            all,
            lang,
            json,
        } => {
            let pref = lang.pref();
            let vn = resolve_vn(&client, &query, &pref)?;
            let title = lang::display_title(&vn, &pref);
            let w = links::lookup(&client, &vn, &title, &pref, prices.as_deref())?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({"title": title, "where": w}))?
                );
            } else {
                println!("{title}  ({})", vn.id);
                render::print_where(&w, "  ", if all { 0 } else { 6 });
            }
        }
    }
    Ok(())
}
