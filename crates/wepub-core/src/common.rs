//! Internal helpers shared across store backends (firefox, chrome, edge).

use url::Url;

use crate::{Phase, Result, Store, WepubError};

/// Parse a user-supplied root URL string and normalize it for use as a
/// base URL (i.e. ensure it ends with a trailing slash so that relative
/// paths join correctly).
///
/// Returns [`WepubError::InvalidUrl`] if the input does not parse.
pub(crate) fn parse_root_url(root_url: &str) -> Result<Url> {
    let mut parsed =
        Url::parse(root_url).map_err(|e| WepubError::InvalidUrl(format!("{root_url:?}: {e}")))?;
    if !parsed.path().ends_with('/') {
        let new_path = format!("{}/", parsed.path());
        parsed.set_path(&new_path);
    }
    Ok(parsed)
}

/// Join `path` onto `root`, mapping URL join errors to
/// [`WepubError::Internal`] (the path is constructed by `wepub-core`
/// itself, so a failure here indicates a bug rather than user input).
pub(crate) fn join_endpoint(root: &Url, path: &str) -> Result<Url> {
    root.join(path)
        .map_err(|e| WepubError::Internal(format!("invalid endpoint path {path:?}: {e}")))
}

/// Drain `resp` into a typed body, logging the raw response at debug
/// level. Non-2xx responses become [`WepubError::HttpStatus`]; bodies
/// that cannot be decoded as `T` become
/// [`WepubError::UnexpectedResponse`] tagged with the supplied `store` /
/// `phase` so callers can locate the failure without inspecting the
/// detail string.
pub(crate) async fn decode_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
    store: Store,
    phase: Phase,
) -> Result<T> {
    let status = resp.status();
    let body = resp.text().await?;
    tracing::debug!(
        store = %store,
        phase = %phase,
        status = status.as_u16(),
        body = %body,
        "received store API response",
    );
    if !status.is_success() {
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_str(&body).map_err(|e| WepubError::UnexpectedResponse {
        store,
        phase,
        detail: format!("failed to decode response: {e}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_root_url_appends_trailing_slash_when_missing() {
        let url = parse_root_url("https://example.com/api/v5").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/v5/");
    }

    #[test]
    fn parse_root_url_preserves_existing_trailing_slash() {
        let url = parse_root_url("https://example.com/api/v5/").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/v5/");
    }

    #[test]
    fn parse_root_url_rejects_garbage() {
        let err = parse_root_url("not a url").unwrap_err();
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    #[test]
    fn join_endpoint_appends_relative_path() {
        let root = Url::parse("https://example.com/").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/v5/addons/upload/");
    }
}
