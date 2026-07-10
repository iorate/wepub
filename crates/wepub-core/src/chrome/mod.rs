//! Chrome Web Store API (v2) client.
//!
//! The entry point is [`publish`]: configure the returned builder, then
//! finish with `call()` to publish an extension. [`fetch_access_token`]
//! exchanges an OAuth refresh token for the access token that [`publish`]
//! requires.

mod api;
mod auth;

pub use api::{FetchAccessTokenBuilder, PublishBuilder, PublishType, fetch_access_token, publish};
