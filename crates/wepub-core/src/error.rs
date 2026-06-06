use std::time::Duration;

use serde_json::json;
use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WepubError {
    /// The underlying HTTP request failed.
    #[error("{}", json!({ "name": "Http", "source": source.to_string() }))]
    Http {
        /// The underlying HTTP error.
        #[from]
        source: reqwest::Error,
    },

    /// The server returned a non-2xx HTTP status.
    #[error("{}", json!({ "name": "HttpStatus", "status": status.as_u16(), "body": body }))]
    HttpStatus {
        /// HTTP status code.
        status: reqwest::StatusCode,
        /// Response body.
        body: String,
    },

    /// The token endpoint of an OAuth server reported the request as failed.
    #[error("{}", json!({ "name": "OAuthToken", "error": error, "error_description": error_description, "error_uri": error_uri }))]
    OAuthToken {
        /// The error code.
        error: String,
        /// The error description, if any.
        error_description: Option<String>,
        /// The error URI, if any.
        error_uri: Option<String>,
    },

    /// A polling loop timed out.
    #[error("{}", json!({ "name": "PollTimeout", "elapsed": elapsed }))]
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server's response did not match the expected format.
    #[error("{}", json!({ "name": "UnexpectedResponse", "reason": reason }))]
    UnexpectedResponse {
        /// Short description of how the response was unexpected.
        reason: String,
    },

    /// A URL could not be parsed.
    #[error("{}", json!({ "name": "Url", "url": url, "source": source.to_string() }))]
    Url {
        /// The URL that failed to parse.
        url: String,
        /// The underlying URL parsing error.
        source: url::ParseError,
    },

    /// The upload endpoint of Chrome Web Store reported the upload as failed.
    #[error("{}", json!({ "name": "ChromeUpload", "upload_state": upload_state }))]
    ChromeUpload {
        /// Upload state.
        upload_state: String,
    },

    /// The publish endpoint of Chrome Web Store reported the publish as failed.
    #[error("{}", json!({ "name": "ChromePublish", "item_state": item_state }))]
    ChromePublish {
        /// Item state.
        item_state: String,
    },

    /// The upload endpoint of Firefox Add-ons reported the upload as failed.
    #[error("{}", json!({ "name": "FirefoxUpload", "validation": validation }))]
    FirefoxUpload {
        /// Validation results.
        validation: serde_json::Value,
    },

    /// Edge Add-ons reported the operation as failed.
    #[error("{}", json!({ "name": "EdgeApi", "message": message, "error_code": error_code, "errors": errors }))]
    EdgeApi {
        /// Operation message, if any.
        message: Option<String>,
        /// Operation error code, if any.
        error_code: Option<String>,
        /// Operation errors, if any.
        errors: Option<Vec<serde_json::Value>>,
    },
}
