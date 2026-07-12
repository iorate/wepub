//! Asynchronous client library for publishing browser extensions to web stores.
//!
//! `wepub-core` is the engine behind the [`wepub`][wepub-bin] command-line
//! tool. It exposes one `publish` entry point per supported store, returning
//! a builder that runs when awaited directly or finished with `call()`.
//!
//! Currently supported stores:
//!
//! - **Chrome Web Store** via [`chrome::publish`].
//! - **Firefox Add-ons** via [`firefox::publish`].
//! - **Edge Add-ons** via [`edge::publish`].
//!
//! All stores share the [`WepubError`] error type and the [`Result`] alias.
//!
//! The async API is runtime-agnostic: the returned futures can be awaited
//! on any executor, and no Tokio runtime is required. Each `publish` (or
//! `fetch_access_token`) call drives its HTTP I/O on a dedicated background
//! thread that lives for the duration of the call.
//!
//! # Example
//!
//! ```no_run
//! # async fn run() -> Result<(), Box<dyn std::error::Error>> {
//! use wepub_core::firefox::{Channel, publish};
//!
//! let package = std::fs::read("./addon.zip")?;
//! publish()
//!     .addon_id("myaddon@example.com")
//!     .api_key("user:12345:6789")
//!     .api_secret("jwt-secret")
//!     .package(package)
//!     .channel(Channel::Listed)
//!     .await?;
//! # Ok(())
//! # }
//! ```
//!
//! [wepub-bin]: https://crates.io/crates/wepub

#![warn(missing_docs)]

mod common;
mod error;
mod http;
mod multipart;

pub mod chrome;
pub mod edge;
pub mod firefox;

pub use error::{Result, WepubError};
