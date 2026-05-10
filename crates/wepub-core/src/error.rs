use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
///
/// Variants split errors by responsibility: the network layer, the wire
/// protocol, the credentials, the per-store domain (validation / upload /
/// publish), and a catch-all [`Internal`](WepubError::Internal) for
/// "should-never-happen" states.
#[derive(Debug, Error)]
pub enum WepubError {
    /// Underlying transport failure surfaced by `reqwest` (DNS, TCP, TLS,
    /// connect / read / overall timeout, body read error, etc.).
    #[error("network error: {0}")]
    Network(#[from] reqwest::Error),

    /// The remote returned a non-2xx HTTP status. `body` carries the
    /// (possibly empty) response body verbatim.
    #[error("API error (status {status}): {body}")]
    Api {
        /// HTTP status code from the failed response.
        status: u16,
        /// Response body received with the failure status.
        body: String,
    },

    /// The OAuth / JWT credential exchange failed at the protocol level
    /// (e.g. Google's token endpoint returned `invalid_grant`).
    #[error("authentication failed: {0}")]
    Auth(String),

    /// Firefox AMO reported the upload as `valid: false`, or validation
    /// polling exceeded its timeout. `body` is the validation result JSON
    /// (pretty-printed) or a timeout description.
    #[error("AMO validation failed for upload {uuid}: {body}")]
    Validation {
        /// AMO upload UUID returned by `POST /addons/upload/`.
        uuid: String,
        /// Pretty-printed AMO validation result, or a timeout description.
        body: String,
    },

    /// Chrome Web Store reported `uploadState = FAILED` or `NOT_FOUND`, or
    /// upload polling exceeded its timeout.
    #[error("CWS upload failed for item {item_id}: {body}")]
    Upload {
        /// CWS item id whose upload failed.
        item_id: String,
        /// Failure description from the official enum docs, or a timeout
        /// description.
        body: String,
    },

    /// The CWS publish endpoint returned 200 OK but the item state is a
    /// terminal failure (`REJECTED` or `CANCELLED`). The HTTP call itself
    /// succeeded, hence this is not [`Api`](WepubError::Api).
    #[error("publish failed for item {item_id}: {body}")]
    Publish {
        /// CWS item id reported in the publish response.
        item_id: String,
        /// Short description of the terminal state.
        body: String,
    },

    /// JSON deserialization of a response body failed. Indicates the wire
    /// shape diverged from this crate's expectations.
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    /// Local filesystem I/O failed (e.g. could not read the source zip).
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// "Should never happen" programmer-error states: URL join failure,
    /// pre-epoch system clock, JWT encode failure, hard-coded MIME literal
    /// rejected by `mime_str`. Reaching this variant indicates a bug in
    /// `wepub-core` itself.
    #[error("internal error: {0}")]
    Internal(String),
}
