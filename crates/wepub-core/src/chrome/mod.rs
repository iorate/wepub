//! Chrome Web Store Publish API (v2) client.
//!
//! The entry point is [`Client`]. Build it with either an OAuth
//! refresh token ([`Client::new`]) or a pre-fetched access token
//! ([`Client::from_access_token`]), then call [`Client::publish`]
//! to upload a packaged extension and submit it for review.

mod api;
mod auth;

pub use api::{Client, PublishOptions, PublishType};
