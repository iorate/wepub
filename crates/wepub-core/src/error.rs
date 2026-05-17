use std::fmt;
use std::time::Duration;

use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Identifies which store backend a failure originated from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreId {
    /// Chrome Web Store.
    Chrome,
    /// Firefox Add-ons (addons.mozilla.org).
    Firefox,
    /// Edge Add-ons.
    Edge,
}

impl StoreId {
    fn as_str(self) -> &'static str {
        match self {
            StoreId::Chrome => "chrome",
            StoreId::Firefox => "firefox",
            StoreId::Edge => "edge",
        }
    }
}

impl fmt::Display for StoreId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Identifies which logical phase of a publish run a failure originated
/// from. Used by cross-cutting variants ([`WepubError::Timeout`] and
/// [`WepubError::UnexpectedResponse`]) so callers can locate the failure
/// without parsing string detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Uploading the extension archive (and, for Firefox, polling the
    /// validation result).
    Upload,
    /// Submitting the uploaded artefact for publish (and, for Edge,
    /// polling the publish operation status).
    Publish,
    /// Exchanging a refresh token for an access token. Chrome-only.
    TokenRefresh,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Upload => "upload",
            Phase::Publish => "publish",
            Phase::TokenRefresh => "token-refresh",
        }
    }
}

impl fmt::Display for Phase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Error type returned by every fallible call in this crate.
///
/// Variants split errors by responsibility:
///
/// - Transport / HTTP layer failures that can occur during normal
///   operation ([`Network`](WepubError::Network),
///   [`HttpStatus`](WepubError::HttpStatus)).
/// - Cross-cutting failures tagged with [`StoreId`] and [`Phase`]:
///   [`Timeout`](WepubError::Timeout) for polling that ran out of budget,
///   and [`UnexpectedResponse`](WepubError::UnexpectedResponse) for
///   responses that violated the documented wire shape (e.g. malformed
///   JSON, missing required fields, missing headers). The latter
///   "should not happen" against a conforming server.
/// - Local I/O / configuration ([`Io`](WepubError::Io),
///   [`InvalidUrl`](WepubError::InvalidUrl)) and the catch-all
///   [`Internal`](WepubError::Internal) for programmer-error states.
/// - Per-store domain failures prefixed by store name: the HTTP call
///   succeeded but the server reported the publish request as rejected.
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

    /// A polling loop exceeded its budget without reaching a terminal
    /// state.
    #[error("{store} {phase} polling timed out after {elapsed:?}")]
    Timeout {
        /// Which store the timed-out poll targeted.
        store: StoreId,
        /// Which phase the timed-out poll belonged to.
        phase: Phase,
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server returned a response that violated the documented wire
    /// shape: malformed JSON, missing required fields, missing required
    /// headers, or an enum value the API documents as never appearing.
    /// Against a conforming server this should not happen; reaching this
    /// variant points at an API change or a server-side bug. Inspect the
    /// `debug`-level request log for the raw body.
    #[error("unexpected response from {store} during {phase}: {detail}")]
    UnexpectedResponse {
        /// Store whose response was malformed.
        store: StoreId,
        /// Phase during which the malformed response was received.
        phase: Phase,
        /// Short description of the wire-shape violation (e.g. a
        /// `serde_json::Error` message or "missing Location header").
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

    /// Chrome Web Store reported `uploadState = FAILED` for the asynchronous
    /// upload. The official V2 response carries no failure detail, so only
    /// the item id is preserved.
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
