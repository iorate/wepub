//! Firefox Add-ons API (v5) client.
//!
//! The entry point is [`publish`]: configure the returned builder, then
//! finish with `call()` to publish an add-on.

mod api;
mod auth;

pub use api::{Application, Channel, Compatibility, PublishBuilder, VersionRange, publish};
