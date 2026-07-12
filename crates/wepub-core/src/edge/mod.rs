//! Edge Add-ons API (v1.1) client.
//!
//! The entry point is [`publish`]: configure the returned builder, then run
//! it by awaiting the builder directly or by finishing with `call()` to
//! publish an add-on.

mod api;

pub use api::{PublishBuilder, publish};
