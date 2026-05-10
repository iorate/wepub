//! Chrome Web Store Publish API (v2) client.
//!
//! The entry point is [`ChromeStore`]. Build it with either an OAuth refresh
//! token ([`ChromeStore::from_client_credentials`]) or a pre-fetched access
//! token ([`ChromeStore::from_access_token`]), then call
//! [`ChromeStore::publish`] to upload a packaged extension and submit it for
//! review.

mod api;
mod auth;

pub use api::{ChromeStore, PollConfig, PublishOptions, PublishType};
