//! Firefox Add-ons (addons.mozilla.org) Add-on Versions API client.
//!
//! The entry point is [`Client`]. Build it with [`Client::new`], then
//! call [`Client::publish`] to upload a packaged add-on, wait for
//! validation to succeed, and create a new version on the existing add-on.
//!
//! Only existing add-ons can be updated; the very first version of an
//! add-on must be uploaded through the Firefox Add-ons web UI.

mod api;
mod auth;

pub use api::{Application, Channel, Client, Compatibility, PublishOptions, VersionRange};
