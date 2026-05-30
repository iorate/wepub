//! Firefox Add-ons API (v5) client.
//!
//! The entry point is [`Client`]. Build it with [`Client::new`], then
//! call [`Client::publish`] to publish an add-on.

mod api;
mod auth;

pub use api::{
    Application, Channel, Client, Compatibility, Credentials, PublishOptions, VersionRange,
};
