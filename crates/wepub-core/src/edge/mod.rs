//! Edge Add-ons Update REST API (v1.1) client.
//!
//! The entry point is [`Client`]. Build it with the API credentials
//! issued by Microsoft Partner Center ([`Client::new`]), then call
//! [`Client::publish`] to upload a packaged extension and submit it
//! for review.

mod api;

pub use api::{Client, PublishOptions};
