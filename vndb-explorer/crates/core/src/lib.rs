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
pub mod extlinks;
pub mod import;
pub mod lang;
pub mod search;
pub mod shelves;
pub mod tagtree;

pub use catalog::Catalog;
pub use search::{search, SearchRequest, SearchResponse};
