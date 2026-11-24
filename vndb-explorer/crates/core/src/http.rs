//! HTTP clients. Desktop builds use the system TLS stack; Android uses rustls with bundled roots.

use anyhow::Result;
use std::time::Duration;

pub fn user_agent() -> String {
    format!(
        "vndb-explorer/{} (+https://gitlab.com/Asgefka/Ai_test)",
        crate::app_version()
    )
}

pub fn blocking_client(timeout: Option<Duration>) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .user_agent(user_agent())
        .timeout(timeout)
        .build()?)
}
