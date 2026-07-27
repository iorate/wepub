use std::time::Duration;

use thiserror::Error;

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WepubError {
    /// The underlying HTTP request failed.
    #[error("the underlying HTTP request failed")]
    #[non_exhaustive]
    Http {
        /// The underlying HTTP error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The server returned a non-2xx HTTP status.
    #[error("the server returned a non-2xx HTTP status")]
    #[non_exhaustive]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body.
        body: String,
    },

    /// The token endpoint of an OAuth server reported the request as failed.
    #[error("the OAuth token endpoint reported the request as failed")]
    #[non_exhaustive]
    OAuthToken {
        /// The error code.
        error: String,
        /// The error description, if any.
        error_description: Option<String>,
        /// The error URI, if any.
        error_uri: Option<String>,
    },

    /// A polling loop timed out.
    #[error("a polling loop timed out")]
    #[non_exhaustive]
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server's response did not match the expected format.
    #[error("the server's response did not match the expected format")]
    #[non_exhaustive]
    UnexpectedResponse {
        /// Short description of how the response was unexpected.
        reason: String,
    },

    /// The upload endpoint of Chrome Web Store reported the upload as failed.
    #[error("the upload endpoint of Chrome Web Store reported the upload as failed")]
    #[non_exhaustive]
    ChromeUpload {
        /// Upload state.
        upload_state: String,
    },

    /// The publish endpoint of Chrome Web Store reported the publish as failed.
    #[error("the publish endpoint of Chrome Web Store reported the publish as failed")]
    #[non_exhaustive]
    ChromePublish {
        /// Item state.
        item_state: String,
    },

    /// The upload endpoint of Firefox Add-ons reported the upload as failed.
    #[error("the upload endpoint of Firefox Add-ons reported the upload as failed")]
    #[non_exhaustive]
    FirefoxUpload {
        /// Validation results.
        validation: serde_json::Value,
    },

    /// Edge Add-ons reported the operation as failed.
    #[error("Edge Add-ons reported the operation as failed")]
    #[non_exhaustive]
    EdgeApi {
        /// Operation message, if any.
        message: Option<String>,
        /// Operation error code, if any.
        error_code: Option<String>,
        /// Operation errors, if any.
        errors: Option<Vec<serde_json::Value>>,
    },
}

impl WepubError {
    pub(crate) fn http(source: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Self {
        Self::Http {
            source: source.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    // `Display` is a static per-variant summary that carries no field values;
    // detail lives in the variant's fields and in `crate::tracing::record_error`.
    #[test]
    fn display_is_a_static_summary_without_field_values() {
        assert_eq!(
            WepubError::HttpStatus {
                status: 404,
                body: "<html>".to_string(),
            }
            .to_string(),
            "the server returned a non-2xx HTTP status",
        );
        assert_eq!(
            WepubError::ChromeUpload {
                upload_state: "FAILED".to_string(),
            }
            .to_string(),
            "the upload endpoint of Chrome Web Store reported the upload as failed",
        );
    }

    // The underlying error is reachable through `source()` but not embedded in
    // `Display`, so the CLI's `{:#}` chain prints it exactly once.
    #[test]
    fn http_keeps_the_underlying_error_as_source_only() {
        let err = WepubError::Http {
            source: "connection reset".into(),
        };
        assert_eq!(err.to_string(), "the underlying HTTP request failed");
        assert!(err.source().is_some());

        let err = WepubError::PollTimeout {
            elapsed: Duration::from_secs(1),
        };
        assert!(err.source().is_none());
    }
}
