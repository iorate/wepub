use std::fmt;
use std::time::Duration;

use serde::{Serialize, Serializer};

/// Convenience alias for [`std::result::Result`] specialized to [`WepubError`].
pub type Result<T> = std::result::Result<T, WepubError>;

/// Error type returned by every fallible call in this crate.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum WepubError {
    /// The underlying HTTP request failed.
    Http {
        /// The underlying HTTP error.
        #[serde(serialize_with = "serialize_display")]
        source: reqwest::Error,
    },

    /// The server returned a non-2xx HTTP status.
    HttpStatus {
        /// HTTP status code.
        status: u16,
        /// Response body.
        body: String,
    },

    /// The token endpoint of an OAuth server reported the request as failed.
    OAuthToken {
        /// The error code.
        error: String,
        /// The error description, if any.
        error_description: Option<String>,
        /// The error URI, if any.
        error_uri: Option<String>,
    },

    /// A polling loop timed out.
    PollTimeout {
        /// Total elapsed time before giving up.
        elapsed: Duration,
    },

    /// The server's response did not match the expected format.
    UnexpectedResponse {
        /// Short description of how the response was unexpected.
        reason: String,
    },

    /// A URL could not be parsed.
    Url {
        /// The URL that failed to parse.
        url: String,
        /// The underlying URL parsing error.
        #[serde(serialize_with = "serialize_display")]
        source: url::ParseError,
    },

    /// The upload endpoint of Chrome Web Store reported the upload as failed.
    ChromeUpload {
        /// Upload state.
        upload_state: String,
    },

    /// The publish endpoint of Chrome Web Store reported the publish as failed.
    ChromePublish {
        /// Item state.
        item_state: String,
    },

    /// The upload endpoint of Firefox Add-ons reported the upload as failed.
    FirefoxUpload {
        /// Validation results.
        validation: serde_json::Value,
    },

    /// Edge Add-ons reported the operation as failed.
    EdgeApi {
        /// Operation message, if any.
        message: Option<String>,
        /// Operation error code, if any.
        error_code: Option<String>,
        /// Operation errors, if any.
        errors: Option<Vec<serde_json::Value>>,
    },
}

impl fmt::Display for WepubError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match serde_json::to_string(self) {
            Ok(json) => f.write_str(&json),
            Err(_) => f.write_str("null"),
        }
    }
}

impl std::error::Error for WepubError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            WepubError::Http { source } => Some(source),
            WepubError::Url { source, .. } => Some(source),
            _ => None,
        }
    }
}

impl From<reqwest::Error> for WepubError {
    fn from(source: reqwest::Error) -> Self {
        WepubError::Http { source }
    }
}

fn serialize_display<T, S>(value: &T, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    T: fmt::Display,
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use super::*;
    use serde_json::json;

    // `Display` is the machine-readable contract (one compact JSON object per
    // error, tagged by `type`); pin its shape so renames or field changes that
    // alter the output are caught here.
    #[test]
    fn display_tags_by_type_with_numeric_status() {
        let err = WepubError::HttpStatus {
            status: 404,
            body: "<html>".to_string(),
        };
        assert_eq!(
            err.to_string(),
            r#"{"type":"HttpStatus","status":404,"body":"<html>"}"#,
        );
    }

    #[test]
    fn display_renders_optional_fields_as_null() {
        let err = WepubError::OAuthToken {
            error: "invalid_grant".to_string(),
            error_description: Some("revoked".to_string()),
            error_uri: None,
        };
        assert_eq!(
            err.to_string(),
            r#"{"type":"OAuthToken","error":"invalid_grant","error_description":"revoked","error_uri":null}"#,
        );
    }

    #[test]
    fn display_stringifies_the_underlying_source() {
        let err = WepubError::Url {
            url: "not a url".to_string(),
            source: url::ParseError::RelativeUrlWithoutBase,
        };
        assert_eq!(
            err.to_string(),
            r#"{"type":"Url","url":"not a url","source":"relative URL without a base"}"#,
        );
    }

    #[test]
    fn display_embeds_nested_json_values() {
        let err = WepubError::FirefoxUpload {
            validation: json!({ "messages": ["manifest broken"] }),
        };
        assert_eq!(
            err.to_string(),
            r#"{"type":"FirefoxUpload","validation":{"messages":["manifest broken"]}}"#,
        );
    }

    #[test]
    fn source_exposes_the_underlying_url_error() {
        let err = WepubError::Url {
            url: "not a url".to_string(),
            source: url::ParseError::RelativeUrlWithoutBase,
        };
        assert!(err.source().is_some());

        let err = WepubError::PollTimeout {
            elapsed: Duration::from_secs(1),
        };
        assert!(err.source().is_none());
    }
}
