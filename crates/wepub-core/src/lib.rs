//! Asynchronous client library for publishing browser extensions to web stores.
//!
//! `wepub-core` is the engine behind the [`wepub`][wepub-bin] command-line
//! tool. It exposes one `publish` entry point per supported store, returning
//! a builder that runs when `.await`ed.
//!
//! Currently supported stores:
//!
//! - **Chrome Web Store** via [`chrome::publish`].
//! - **Firefox Add-ons** via [`firefox::publish`].
//! - **Edge Add-ons** via [`edge::publish`].
//!
//! All stores share the [`WepubError`] error type and the [`Result`] alias.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use wepub_core::firefox::{Channel, Credentials};
//!
//! let zip = std::fs::read("./addon.zip")?;
//! wepub_core::firefox::publish(
//!     "myaddon@example.com".into(),
//!     Credentials {
//!         api_key: "user:12345:6789".into(),
//!         api_secret: "jwt-secret".into(),
//!     },
//!     zip,
//!     Channel::Listed,
//! )
//! .await?;
//! # Ok(())
//! # }
//! ```
//!
//! [wepub-bin]: https://crates.io/crates/wepub

#![warn(missing_docs)]

mod common;
mod error;
mod http;

pub mod chrome;
pub mod edge;
pub mod firefox;

pub use error::{Result, WepubError};
