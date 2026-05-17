//! Microsoft Edge Add-ons Update REST API (v1.1) client.
//!
//! The entry point is [`EdgeStore`]. Build it with the API credentials
//! issued by Microsoft Partner Center
//! ([`EdgeStore::from_api_credentials`]), then call [`EdgeStore::publish`]
//! to upload a packaged extension and submit it for review.

mod api;

pub use api::{EdgePollConfig, EdgePublishOptions, EdgeStore};
