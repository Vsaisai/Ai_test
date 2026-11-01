//! vndb-web as a library: the REST API + embedded web UI, usable from the desktop binary
//! and from the Android app (which starts it on a local port and shows it in a WebView).

pub mod api;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

/// Start the server on `127.0.0.1:<random port>` in a background thread and return the port.
/// Used by mobile shells.
pub fn spawn_local(data_dir: PathBuf) -> anyhow::Result<u16> {
    std::fs::create_dir_all(&data_dir)?;
    let (tx, rx) = std::sync::mpsc::channel::<anyhow::Result<u16>>();
    std::thread::Builder::new()
        .name("vndb-web".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = tx.send(Err(e.into()));
                    return;
                }
            };
            rt.block_on(async move {
                let started = async {
                    let state = Arc::new(api::AppState::new(data_dir)?);
                    if state.ready() {
                        api::maybe_refresh_sources(state.clone());
                    }
                    let listener =
                        tokio::net::TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
                            .await?;
                    let port = listener.local_addr()?.port();
                    anyhow::Ok((state, listener, port))
                }
                .await;
                match started {
                    Ok((state, listener, port)) => {
                        let _ = tx.send(Ok(port));
                        let _ = axum::serve(listener, api::router(state)).await;
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            });
        })?;
    rx.recv()?
}
