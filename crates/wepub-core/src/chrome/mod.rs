//! Chrome Web Store API (v2) client.
//!
//! The entry point is [`publish`]: configure the returned [`Publish`]
//! builder, then `.await` it to publish an extension.

mod api;
mod auth;

pub use api::{Credentials, Publish, PublishType, publish};
