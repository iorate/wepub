//! Firefox Add-ons API (v5) client.
//!
//! The entry point is [`publish`]: configure the returned [`Publish`]
//! builder, then `.await` it to publish an add-on.

mod api;
mod auth;

pub use api::{Application, Channel, Compatibility, Credentials, Publish, VersionRange, publish};
