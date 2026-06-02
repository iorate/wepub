use std::time::Duration;

use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WepubError {
    /// The server returned a non-2xx HTTP status.
    #[error("HTTP error (status {status}): {body}")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body, as received.
        body: String,
    },

    /// An invalid URL was provided.
    #[error("invalid URL: {0}")]
    InvalidUrl(String),

    /// Local filesystem I/O failed.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// An error from the underlying HTTP client.
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// A polling loop exceeded the timeout without reaching a terminal state.
    #[error("polling timed out after {elapsed:?}")]
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server returned a response that violated the documented wire
    /// format.
    #[error("unexpected response: {detail}")]
    UnexpectedResponse {
        /// Short description of the wire-format violation.
        detail: String,
    },

    /// Chrome Web Store reported the upload as failed.
    #[error("upload failed for item {item_id}: {reason}")]
    ChromeUploadFailed {
        /// Item id.
        item_id: String,
        /// Short human-readable cause.
        reason: String,
    },

    /// Chrome Web Store reported the publish as failed.
    #[error("publish failed for item {item_id}: {reason}")]
    ChromePublishFailed {
        /// Item id.
        item_id: String,
        /// Short human-readable cause.
        reason: String,
    },

    /// Firefox Add-ons reported the upload as having failed validation.
    #[error("validation failed for upload {upload_uuid}: {validation}")]
    FirefoxValidationFailed {
        /// Upload UUID.
        upload_uuid: String,
        /// Validation results, pretty-printed.
        validation: String,
    },

    /// Edge Add-ons reported the upload as failed.
    #[error("upload failed for product {product_id}: {operation}")]
    EdgeUploadFailed {
        /// Product id.
        product_id: String,
        /// Operation status response, pretty-printed.
        operation: String,
    },

    /// Edge Add-ons reported the publish as failed.
    #[error("publish failed for product {product_id}: {operation}")]
    EdgePublishFailed {
        /// Product id.
        product_id: String,
        /// Operation status response, pretty-printed.
        operation: String,
    },
}
