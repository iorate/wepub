//! Firefox Add-ons (addons.mozilla.org) Add-on Versions API client.
//!
//! The entry point is [`Store`]. Build it with
//! [`Store::from_credentials`], then call
//! [`Store::publish`] to upload a packaged add-on, wait for
//! validation to succeed, and create a new version on the existing add-on.
//!
//! Only existing add-ons can be updated; the very first version of an
//! add-on must be uploaded through the Firefox Add-ons web UI.

mod api;
mod auth;

pub use api::{
    Application, Channel, Compatibility, PollConfig, PublishOptions, Store, VersionRange,
};
