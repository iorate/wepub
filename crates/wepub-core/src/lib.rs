//! Asynchronous client library for publishing browser extensions to web stores.
//!
//! `wepub-core` is the engine behind the [`wepub`][wepub-bin] command-line
//! tool. It exposes one client type per supported store and a single
//! [`publish`][publish-fn] verb that handles upload, validation polling and
//! version submission end-to-end.
//!
//! Currently supported stores:
//!
//! - **Chrome Web Store** via [`chrome::Store`]. Uses an OAuth refresh
//!   token (or a pre-fetched access token).
//! - **Firefox Add-ons** via [`firefox::Store`]. Uses an HS256 JWT
//!   credential pair.
//! - **Edge Add-ons** via [`edge::Store`]. Uses the Partner Center API
//!   key + Client ID credential pair (v1.1).
//!
//! All stores share the [`WepubError`] error type and the [`Result`] alias.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> wepub_core::Result<()> {
//! use wepub_core::firefox::{PublishOptions, Store};
//!
//! let store = Store::from_credentials(
//!     "myaddon@example.com".into(),
//!     "user:12345:6789".into(),
//!     "jwt-secret".into(),
//! )?;
//! let zip = std::fs::read("./addon.zip")?;
//! store.publish(zip, PublishOptions::default()).await?;
//! # Ok(())
//! # }
//! ```
//!
//! [wepub-bin]: https://crates.io/crates/wepub
//! [publish-fn]: firefox::Store::publish

#![warn(missing_docs)]

mod common;
mod error;
mod http;

pub mod chrome;
pub mod edge;
pub mod firefox;

pub use error::{Phase, Result, StoreId, WepubError};
