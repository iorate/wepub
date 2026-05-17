use std::time::Duration;

use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
///
/// Variants split errors by responsibility:
///
/// - Transport / HTTP layer failures that can occur during normal
///   operation ([`Network`](WepubError::Network),
///   [`HttpStatus`](WepubError::HttpStatus)).
/// - Cross-cutting failures:
///   [`PollTimeout`](WepubError::PollTimeout) for a polling loop that
///   exhausted its per-store `PollConfig::timeout` budget, and
///   [`UnexpectedResponse`](WepubError::UnexpectedResponse) for responses
///   that violated the documented wire shape (e.g. malformed JSON,
///   missing required fields, missing headers). The latter "should not
///   happen" against a conforming server. The store and the operation
///   being attempted at the time are visible in the `tracing` log
///   stream immediately preceding the failure.
/// - Local I/O / configuration ([`Io`](WepubError::Io),
///   [`InvalidUrl`](WepubError::InvalidUrl)) and the catch-all
///   [`Internal`](WepubError::Internal) for programmer-error states.
/// - Per-store domain failures prefixed by store name: the HTTP call
///   succeeded but the server reported the upload or the submission as
///   rejected.
#[derive(Debug, Error)]
pub enum WepubError {
    /// Underlying transport failure surfaced by `reqwest` (DNS, TCP, TLS,
    /// connect / read / overall timeout, body read error, etc.).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The remote returned a non-2xx HTTP status. `body` carries the
    /// (possibly empty) response body verbatim.
    #[error("HTTP error (status {status}): {body}")]
    HttpStatus {
        /// HTTP status code from the failed response.
        status: u16,
        /// Response body received with the failure status.
        body: String,
    },

    /// A polling loop exceeded its per-store `PollConfig::timeout` budget
    /// without reaching a terminal state. The preceding `tracing` log
    /// identifies which poll (e.g. `polling Firefox Add-ons upload status`)
    /// was in flight.
    #[error("polling timed out after {elapsed:?}")]
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server returned a response that violated the documented wire
    /// shape: malformed JSON, missing required fields, missing required
    /// headers, or an enum value the API documents as never appearing.
    /// Against a conforming server this should not happen; reaching this
    /// variant points at an API change or a server-side bug. Inspect the
    /// `debug`-level request log for the raw body, and the preceding
    /// `info`-level log for which operation was in flight.
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

    /// "Should never happen" programmer-error states: URL join failure,
    /// pre-epoch system clock, JWT encode failure, hard-coded MIME literal
    /// rejected by `mime_str`. Reaching this variant indicates a bug in
    /// `wepub-core` itself.
    #[error("internal error: {0}")]
    Internal(String),

    /// Chrome Web Store reported the asynchronous upload as failed. The
    /// V2 response carries no failure detail, so only the item id is
    /// preserved.
    #[error("chrome upload failed for item {item_id}")]
    ChromeUploadFailed {
        /// Chrome Web Store item id whose upload failed.
        item_id: String,
    },

    /// The Chrome Web Store `:publish` endpoint returned 200 OK but the item
    /// reached a terminal failure state (`REJECTED` or `CANCELLED`).
    /// `detail` is the pretty-printed publish response.
    #[error("chrome publish failed for item {item_id}: {detail}")]
    ChromePublishFailed {
        /// Chrome Web Store item id reported in the publish response.
        item_id: String,
        /// Pretty-printed Chrome Web Store publish response body.
        detail: String,
    },

    /// Firefox Add-ons reported the upload as `valid: false`. `detail` is
    /// the pretty-printed `validation` JSON tree returned by the API.
    #[error("firefox validation failed for upload {uuid}: {detail}")]
    FirefoxValidationFailed {
        /// Firefox Add-ons upload UUID returned by `POST /addons/upload/`.
        uuid: String,
        /// Pretty-printed Firefox Add-ons `validation` field.
        detail: String,
    },

    /// The Edge upload operation reached `status: "Failed"`. `detail` is
    /// the pretty-printed operation response (carrying `message`,
    /// `errorCode`, `errors`, ...).
    #[error("edge upload failed for product {product_id}: {detail}")]
    EdgeUploadFailed {
        /// Edge product id whose upload failed.
        product_id: String,
        /// Pretty-printed Edge upload operation response.
        detail: String,
    },

    /// The Edge publish operation reached `status: "Failed"` (or the
    /// documented "unexpected failure" shape where `status` is absent).
    /// `detail` is the pretty-printed operation response.
    #[error("edge publish failed for product {product_id}: {detail}")]
    EdgePublishFailed {
        /// Edge product id whose publish failed.
        product_id: String,
        /// Pretty-printed Edge publish operation response.
        detail: String,
    },
}
