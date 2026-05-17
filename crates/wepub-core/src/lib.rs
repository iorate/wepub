//! Asynchronous client library for publishing browser extensions to web stores.
//!
//! `wepub-core` is the engine behind the [`wepub`][wepub-bin] command-line
//! tool. It exposes one client type per supported store and a single
//! [`publish`][publish-fn] verb that handles upload, validation polling and
//! version submission end-to-end.
//!
//! Currently supported stores:
//!
//! - **Firefox Add-ons** via [`firefox::FirefoxStore`]. Uses an HS256 JWT
//!   credential pair.
//! - **Chrome Web Store** via [`chrome::ChromeStore`]. Uses an OAuth refresh
//!   token (or a pre-fetched access token).
//! - **Edge Add-ons** via [`edge::EdgeStore`]. Uses the Partner Center API
//!   key + Client ID credential pair (v1.1).
//!
//! All stores share the [`WepubError`] error type and the [`Result`] alias.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> wepub_core::Result<()> {
//! use wepub_core::firefox::{FirefoxPublishOptions, FirefoxStore};
//!
//! let store = FirefoxStore::from_credentials(
//!     "myaddon@example.com".into(),
//!     "user:12345:6789".into(),
//!     "jwt-secret".into(),
//! )?;
//! let zip = std::fs::read("./addon.zip")?;
//! store.publish(zip, FirefoxPublishOptions::default()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! [wepub-bin]: https://crates.io/crates/wepub
//! [publish-fn]: firefox::FirefoxStore::publish

#![warn(missing_docs)]

mod common;
mod error;
mod http;

pub mod chrome;
pub mod edge;
pub mod firefox;

pub use error::{Phase, Result, Store, WepubError};
