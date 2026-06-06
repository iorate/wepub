//! Asynchronous client library for publishing browser extensions to web stores.
//!
//! `wepub-core` is the engine behind the [`wepub`][wepub-bin] command-line
//! tool. It exposes one `Client` type per supported store and a single
//! `publish` verb.
//!
//! Currently supported stores:
//!
//! - **Chrome Web Store** via [`chrome::Client`].
//! - **Firefox Add-ons** via [`firefox::Client`].
//! - **Edge Add-ons** via [`edge::Client`].
//!
//! All stores share the [`WepubError`] error type and the [`Result`] alias.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use wepub_core::firefox::{Channel, Client, Credentials, PublishOptions};
//!
//! let client = Client::new(
//!     "myaddon@example.com".into(),
//!     Credentials {
//!         api_key: "user:12345:6789".into(),
//!         api_secret: "jwt-secret".into(),
//!     },
//! )?;
//! let zip = std::fs::read("./addon.zip")?;
//! client.publish(zip, Channel::Listed, PublishOptions::new()).await?;
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

pub use common::PollConfig;
pub use error::{Result, WepubError};
