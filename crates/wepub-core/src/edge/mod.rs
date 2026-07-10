//! Edge Add-ons API (v1.1) client.
//!
//! The entry point is [`publish`]: configure the returned [`Publish`]
//! builder, then `.await` it to publish an add-on.

mod api;

pub use api::{Credentials, Publish, publish};
