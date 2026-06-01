//! Chrome Web Store API (v2) client.
//!
//! The entry point is [`Client`]. Build it with [`Client::new`], then
//! call [`Client::publish`] to publish an extension.

mod api;
mod auth;

pub use api::{Client, Credentials, Progress, PublishOptions, PublishType};
