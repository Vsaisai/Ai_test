//! Offline VNDB catalog.
//!
//! * [`import`] — download the full VNDB database dump and build a local SQLite file
//! * [`catalog::Catalog`] — in-memory index of every VN (18+ included)
//! * [`search`] — tag search with required / wanted / excluded tags and ranking
//! * [`shelves`] — named, categorized searches ("shelves")
//! * [`detail`] — one VN with categorized tags, releases and where-to-get links
//!
//! The crate has no web dependencies so it can be embedded in a server, a CLI or a mobile app.

pub mod catalog;
pub mod copy;
pub mod db;
pub mod detail;
pub mod external;
pub mod extlinks;
pub mod http;
pub mod import;
pub mod lang;
pub mod profile;
pub mod progress;
pub mod search;
pub mod shelves;
pub mod similar;
pub mod sources;
pub mod sync;
pub mod tagtree;
pub mod translate;

/// Release version from the `VERSION` file: `A.B.C` or `A.B.C.D` for small fixes
/// (Cargo's own version only takes three parts).
pub fn app_version() -> &'static str {
    include_str!("../../../VERSION").trim()
}

pub use catalog::Catalog;
pub use search::{search, SearchRequest, SearchResponse};
