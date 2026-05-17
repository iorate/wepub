use std::time::Duration;

use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Error)]
pub enum WepubError {
    /// Underlying transport failure surfaced by `reqwest` (DNS, TCP, TLS,
    /// connect / read / overall timeout, body read error, etc.).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The remote returned a non-2xx HTTP status.
    #[error("HTTP error (status {status}): {body}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body, as received (possibly empty).
        body: String,
    },

    /// A polling loop exceeded its per-store `PollConfig::timeout` budget
    /// without reaching a terminal state.
    #[error("polling timed out after {elapsed:?}")]
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server returned a response that violated the documented wire
    /// shape: malformed JSON, missing required fields, missing required
    /// headers, or an enum value the API documents as never appearing.
    /// Against a conforming server this should not happen; reaching this
    /// variant points at an API change or a server-side bug.
    #[error("unexpected response: {detail}")]
    UnexpectedResponse {
        /// Short description of the wire-shape violation.
        detail: String,
    },

    /// Local filesystem I/O failed (e.g. could not read the source zip).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// A URL passed in by the caller (typically through one of the `with_*`
    /// builders) failed to parse.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// "Should never happen" programmer-error state. Reaching this
    /// variant indicates a bug in `wepub-core` itself.
    #[error("internal error: {0}")]
    Internal(String),

    /// Chrome Web Store reported the asynchronous upload as failed. No
    /// further detail is available from the API.
    #[error("chrome upload failed for item {item_id}")]
    ChromeUploadFailed {
        /// Chrome Web Store item id.
        item_id: String,
    },

    /// Chrome Web Store accepted the publish request but reported the
    /// item as having reached a terminal failure state.
    #[error("chrome publish failed for item {item_id}: {detail}")]
    ChromePublishFailed {
        /// Chrome Web Store item id.
        item_id: String,
        /// Server's failure response, as a human-readable dump.
        detail: String,
    },

    /// Firefox Add-ons reported the upload as having failed validation.
    #[error("firefox validation failed for upload {uuid}: {detail}")]
    FirefoxValidationFailed {
        /// Firefox Add-ons upload UUID.
        uuid: String,
        /// Server's validation report, as a human-readable dump.
        detail: String,
    },

    /// Edge Add-ons reported the upload operation as failed.
    #[error("edge upload failed for product {product_id}: {detail}")]
    EdgeUploadFailed {
        /// Edge product id.
        product_id: String,
        /// Server's failure response, as a human-readable dump.
        detail: String,
    },

    /// Edge Add-ons reported the publish operation as failed (including
    /// the documented "unexpected failure" response shape).
    #[error("edge publish failed for product {product_id}: {detail}")]
    EdgePublishFailed {
        /// Edge product id.
        product_id: String,
        /// Server's failure response, as a human-readable dump.
        detail: String,
    },
}
