//! Edge Add-ons Update REST API (v1.1) client.
//!
//! The entry point is [`Store`]. Build it with the API credentials
//! issued by Microsoft Partner Center
//! ([`Store::from_credentials`]), then call [`Store::publish`]
//! to upload a packaged extension and submit it for review.

mod api;

pub use api::{PublishOptions, Store};
