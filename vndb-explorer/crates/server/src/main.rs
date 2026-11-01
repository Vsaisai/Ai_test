//! vndb-web: local web UI + REST API over the offline VNDB catalog.
//!
//!   vndb-web import            download the full VNDB dump and build the local database
//!   vndb-web serve             run the web interface (default http://127.0.0.1:8080)

mod api;

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
    /// Start the web interface
    Serve {
        /// Address to listen on. Use 0.0.0.0:8080 to open it from a phone in the same network
        #[arg(long, default_value = "127.0.0.1:8080")]
        addr: SocketAddr,
        /// Start downloading the database right away if there is none yet
        #[arg(long)]
        auto_import: bool,
    },
}

fn default_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("vndb-explorer")
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let data = cli.data.unwrap_or_else(default_data_dir);
    std::fs::create_dir_all(&data)?;
    match cli.cmd.unwrap_or(Cmd::Serve {
        addr: "127.0.0.1:8080".parse()?,
        auto_import: false,
    }) {
        Cmd::Import { file } => {
            let db = data.join("catalog.sqlite");
            let src = match &file {
                Some(p) => vndb_core::import::Source::File(p),
                None => vndb_core::import::Source::Url(vndb_core::import::DUMP_URL),
            };
            let t = std::time::Instant::now();
            let stats = vndb_core::import::import(src, &db, &|m| eprintln!("{m}"))?;
            eprintln!(
                "Imported in {:.0?}: {}",
                t.elapsed(),
                serde_json::to_string(&stats)?
            );
            eprintln!("Database: {}", db.display());
            Ok(())
        }
        Cmd::Serve { addr, auto_import } => {
            let rt = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()?;
            rt.block_on(async move {
                let state = Arc::new(api::AppState::new(data)?);
                if auto_import && !state.ready() {
                    api::start_import(state.clone(), None);
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
