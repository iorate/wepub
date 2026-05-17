use url::Url;

use crate::{Result, WepubError};

// A trailing slash is required so `Url::join` appends relative paths
// instead of replacing the last path segment.
pub(crate) fn parse_root_url(root_url: &str) -> Result<Url> {
    let mut parsed =
        Url::parse(root_url).map_err(|e| WepubError::InvalidUrl(format!("{root_url:?}: {e}")))?;
    if !parsed.path().ends_with('/') {
        let new_path = format!("{}/", parsed.path());
        parsed.set_path(&new_path);
    }
    Ok(parsed)
}

// `path` is always a crate-internal literal, so a join failure is a
// bug, not user input. Map it to `Internal` rather than `InvalidUrl`.
pub(crate) fn join_endpoint(root: &Url, path: &str) -> Result<Url> {
    root.join(path)
        .map_err(|e| WepubError::Internal(format!("invalid endpoint path {path:?}: {e}")))
}

pub(crate) fn log_request(method: &reqwest::Method, url: &reqwest::Url) {
    tracing::debug!(
        method = %method,
        url = %url,
        "sending request",
    );
}

pub(crate) async fn decode_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T> {
    let status = resp.status();
    let body = resp.text().await?;
    tracing::debug!(
        status = status.as_u16(),
        body = %body,
        "received response",
    );
    if !status.is_success() {
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_str(&body).map_err(|e| WepubError::UnexpectedResponse {
        detail: format!("failed to decode response: {e}"),
    })
}

pub(crate) fn pretty_json<T: serde::Serialize + std::fmt::Debug>(value: &T) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| format!("{value:?}"))
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
