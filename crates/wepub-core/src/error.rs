use std::time::Duration;

use thiserror::Error;
use tracing::{Level, debug, error, info, trace, warn};

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum WepubError {
    /// The underlying HTTP request failed.
    #[error("the underlying HTTP request failed")]
    Http {
        /// The underlying HTTP error.
        source: Box<dyn std::error::Error + Send + Sync>,
    },

    /// The server returned a non-2xx HTTP status.
    #[error("the server returned a non-2xx HTTP status")]
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body.
        body: String,
    },

    /// The token endpoint of an OAuth server reported the request as failed.
    #[error("the OAuth token endpoint reported the request as failed")]
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
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server's response did not match the expected format.
    #[error("the server's response did not match the expected format")]
    UnexpectedResponse {
        /// Short description of how the response was unexpected.
        reason: String,
    },

    /// The upload endpoint of Chrome Web Store reported the upload as failed.
    #[error("the upload endpoint of Chrome Web Store reported the upload as failed")]
    ChromeUpload {
        /// Upload state.
        upload_state: String,
    },

    /// The publish endpoint of Chrome Web Store reported the publish as failed.
    #[error("the publish endpoint of Chrome Web Store reported the publish as failed")]
    ChromePublish {
        /// Item state.
        item_state: String,
    },

    /// The upload endpoint of Firefox Add-ons reported the upload as failed.
    #[error("the upload endpoint of Firefox Add-ons reported the upload as failed")]
    FirefoxUpload {
        /// Validation results.
        validation: serde_json::Value,
    },

    /// Edge Add-ons reported the operation as failed.
    #[error("Edge Add-ons reported the operation as failed")]
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
    pub(crate) fn http(source: reqwest::Error) -> Self {
        Self::Http {
            source: Box::new(source),
        }
    }
}

pub(crate) fn record_error(level: Level, err: &WepubError) {
    macro_rules! record {
        ($($args:tt)*) => {
            match level {
                Level::TRACE => trace!($($args)*),
                Level::DEBUG => debug!($($args)*),
                Level::INFO => info!($($args)*),
                Level::WARN => warn!($($args)*),
                Level::ERROR => error!($($args)*),
            }
        };
    }
    match err {
        WepubError::Http { source } => {
            let source = source.to_string();
            record!(source = source.as_str(), "{err}");
        }
        WepubError::HttpStatus { status, body } => {
            record!(status = *status, body = body.as_str(), "{err}");
        }
        WepubError::OAuthToken {
            error,
            error_description,
            error_uri,
        } => {
            record!(
                error = error.as_str(),
                error_description = error_description.as_deref(),
                error_uri = error_uri.as_deref(),
                "{err}",
            );
        }
        WepubError::PollTimeout { elapsed } => {
            record!(elapsed_secs = elapsed.as_secs_f64(), "{err}");
        }
        WepubError::UnexpectedResponse { reason } => {
            record!(reason = reason.as_str(), "{err}");
        }
        WepubError::ChromeUpload { upload_state } => {
            record!(upload_state = upload_state.as_str(), "{err}");
        }
        WepubError::ChromePublish { item_state } => {
            record!(item_state = item_state.as_str(), "{err}");
        }
        WepubError::FirefoxUpload { validation } => {
            let validation = serde_json::to_string(validation).unwrap_or_default();
            record!(validation = validation.as_str(), "{err}");
        }
        WepubError::EdgeApi {
            message,
            error_code,
            errors,
        } => {
            let errors = errors
                .as_ref()
                .map(|errors| serde_json::to_string(errors).unwrap_or_default());
            record!(
                message = message.as_deref(),
                error_code = error_code.as_deref(),
                errors = errors.as_deref(),
                "{err}",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;

    // `Display` is a static per-variant summary that carries no field values;
    // detail lives in the variant's fields and in `record_error`.
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
