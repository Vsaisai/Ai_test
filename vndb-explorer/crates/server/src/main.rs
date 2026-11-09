//! vndb-web: local web UI + REST API over the offline VNDB catalog.
//!
//!   vndb-web import            download the full VNDB dump and build the local database
//!   vndb-web serve             run the web interface (default http://127.0.0.1:8080)

use vndb_web::api;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "vndb-web",
    version,
    about = "Local VNDB catalog with a web interface"
)]
struct Cli {
    /// Data directory (database, personal lists, image cache)
    #[arg(long, global = true, env = "VNDB_DATA")]
    data: Option<PathBuf>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Download the VNDB database dump and (re)build the local catalog
    Import {
        /// Use an already downloaded vndb-db-*.tar.zst instead of downloading
        #[arg(long)]
        file: Option<PathBuf>,
    },
    /// Fetch new visual novels and releases through the VNDB API (fast, small)
    Sync {
        /// Only report how far behind the local database is
        #[arg(long)]
        check: bool,
    },
    /// Refresh third-party catalog indexes (vngames.ru ...) and report matches
    Sources,
    /// A short test of the F95Zone catalog against the real site (about 7 requests, 3 s apart):
    /// feed, Games forum list and a few game pages. Results go to external.sqlite.
    F95Check {
        /// Game pages to open
        #[arg(long, default_value_t = 3)]
        games: usize,
    },
    /// Start the web interface
    Serve {
        /// Address to listen on. Use 0.0.0.0:8080 to open it from a phone in the same network
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
        /// Start downloading the database right away if there is none yet
        #[arg(long)]
        auto_import: bool,
        /// Serve the web UI from this folder (default: the copy built into the program)
        #[arg(long, value_name = "DIR")]
        web: Option<PathBuf>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data = cli.data.unwrap_or_else(vndb_core::db::default_data_dir);
    std::fs::create_dir_all(&data)?;
    match cli.cmd.unwrap_or(Cmd::Serve {
        addr: "127.0.0.1:8080".parse()?,
        auto_import: false,
        web: None,
    }) {
        Cmd::Import { file } => {
            let db = data.join("catalog.sqlite");
            let src = match &file {
                Some(p) => vndb_core::import::Source::File(p),
                None => vndb_core::import::Source::Url(vndb_core::import::DUMP_URL),
            };
            let t = std::time::Instant::now();
            let stats = vndb_core::import::import(
                src,
                &db,
                &vndb_core::progress::Reporter::text(&|m: &str| eprintln!("{m}")),
            )?;
            eprintln!(
                "Imported in {:.0?}: {}",
                t.elapsed(),
                serde_json::to_string(&stats)?
            );
            eprintln!("Database: {}", db.display());
            Ok(())
        }
        Cmd::Sync { check } => {
            let db = data.join("catalog.sqlite");
            let th = vndb_core::sync::Thresholds::default();
            let c = vndb_core::sync::check(&db, &th)?;
            eprintln!("{}", serde_json::to_string_pretty(&c)?);
            if !check {
                if c.big {
                    eprintln!(
                        "Too far behind ({}), run `vndb-web import` for a full update.",
                        c.reason.unwrap_or_default()
                    );
                } else {
                    let r = vndb_core::sync::apply(
                        &db,
                        &vndb_core::progress::Reporter::text(&|m: &str| eprintln!("{m}")),
                    )?;
                    eprintln!("{}", serde_json::to_string(&r)?);
                }
            }
            Ok(())
        }
        Cmd::F95Check { games } => {
            let conn = vndb_core::external::store::open(&data.join("external.sqlite"))?;
            let r = vndb_core::external::f95zone::live_check(&conn, games.min(10), &|m: &str| {
                println!("{m}")
            })?;
            println!();
            println!(
                "Итог: {}{}",
                if r.ok {
                    "все проверки пройдены"
                } else {
                    "есть проблемы (см. выше)"
                },
                r.stopped
                    .map(|s| format!(" — остановлено: {s}"))
                    .unwrap_or_default()
            );
            if let (Some(f), Some(t)) = (r.feed_releases, r.forum_threads) {
                println!("В ленте {f} релизов, в форуме Games {t} тем.");
            }
            Ok(())
        }
        Cmd::Sources => {
            let defs = vndb_core::sources::Sources::load(&data)?;
            let user = vndb_core::db::open_user(&data.join("user.sqlite"))?;
            let res = vndb_core::sources::refresh_index(&defs, &user, &|m| eprintln!("{m}"))?;
            eprintln!("Indexed: {res:?}");
            let cat = vndb_core::Catalog::open(&data.join("catalog.sqlite"))?;
            let conn = cat.connection()?;
            let mut matched = std::collections::HashSet::new();
            for v in &cat.vns {
                let aliases: String =
                    conn.query_row("SELECT aliases FROM vn WHERE id = ?1", [v.id], |r| r.get(0))?;
                let aliases: Vec<String> = aliases.lines().map(String::from).collect();
                for l in vndb_core::sources::links_for(
                    &defs,
                    &user,
                    v,
                    &aliases,
                    &["en".into()],
                    "community",
                ) {
                    if l.direct {
                        matched.insert(l.url);
                    }
                }
            }
            for (id, n) in res {
                let m = matched
                    .iter()
                    .filter(|u| {
                        u.contains(&id)
                            || defs.sources.iter().any(|s| {
                                s.id == id
                                    && s.index
                                        .as_ref()
                                        .map(|i| u.starts_with(&i.prefix))
                                        .unwrap_or(false)
                            })
                    })
                    .count();
                eprintln!("{id}: {m} of {n} pages matched to VNDB entries");
            }
            Ok(())
        }
        Cmd::Serve {
            addr,
            auto_import,
            web,
        } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let mut state = api::AppState::new(data)?;
                if let Some(dir) = web.filter(|d| d.join("index.html").is_file()) {
                    eprintln!("Web UI from {}", dir.display());
                    state.web_dir = Some(dir);
                }
                let state = Arc::new(state);
                if auto_import && !state.ready() {
                    api::start_import(state.clone(), None);
                } else {
                    api::maybe_refresh_sources(state.clone());
                }
                let app = api::router(state.clone());
                let listener = tokio::net::TcpListener::bind(addr).await?;
                eprintln!("vndb-web is running: http://{addr}/");
                if addr.ip().is_unspecified() {
                    eprintln!(
                        "Open http://<this-computer-ip>:{}/ on your phone (same Wi-Fi).",
                        addr.port()
                    );
                }
                axum::serve(listener, app).await?;
                Ok(())
            })
        }
    }
}
