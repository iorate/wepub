//! Chrome Web Store Publish API (v2) client.
//!
//! The entry point is [`Store`]. Build it with either an OAuth refresh
//! token ([`Store::from_credentials`]) or a pre-fetched access
//! token ([`Store::from_access_token`]), then call
//! [`Store::publish`] to upload a packaged extension and submit it for
//! review.

mod api;
mod auth;

pub use api::{PollConfig, PublishOptions, PublishType, Store};
