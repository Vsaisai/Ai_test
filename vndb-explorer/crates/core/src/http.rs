//! HTTP clients. Desktop builds use the system TLS stack; Android uses rustls with bundled roots.

use anyhow::Result;
use std::time::Duration;

pub const USER_AGENT: &str = concat!(
    "vndb-explorer/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Vsaisai/Ai_test)"
);

pub fn blocking_client(timeout: Option<Duration>) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(timeout)
        .build()?)
}
