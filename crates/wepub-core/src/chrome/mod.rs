//! Chrome Web Store Publish API (v2) client.
//!
//! The entry point is [`Client`]. Build it with [`Client::new`] and the
//! appropriate [`Credentials`] (an OAuth refresh token or a pre-fetched
//! access token), then call [`Client::publish`] to upload a packaged
//! extension and submit it for review.

mod api;
mod auth;

pub use api::{Client, Credentials, PublishOptions, PublishType};
