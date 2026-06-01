//! Edge Add-ons API (v1.1) client.
//!
//! The entry point is [`Client`]. Build it with [`Client::new`], then
//! call [`Client::publish`] to publish an add-on.

mod api;

pub use api::{Client, Credentials, Progress, PublishOptions};
