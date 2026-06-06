use std::time::Duration;

use url::Url;

use crate::{Result, WepubError, error::tracing_error};

/// Polling interval and timeout.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PollConfig {
    /// Delay between successive polls.
    pub interval: Duration,
    /// Maximum total time to wait before giving up.
    pub timeout: Duration,
}

pub(crate) async fn instrument_step<T>(
    span: tracing::Span,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    use tracing::Instrument;
    async move { fut.await.inspect_err(tracing_error) }
        .instrument(span)
        .await
}

pub(crate) fn parse_root_url(root_url: &str) -> Result<Url> {
    let mut parsed = Url::parse(root_url).map_err(|err| WepubError::Url {
        url: root_url.to_string(),
        source: err,
    })?;
    // A trailing slash makes `Url::join` append rather than replace the last segment.
    if !parsed.path().ends_with('/') {
        let new_path = format!("{}/", parsed.path());
        parsed.set_path(&new_path);
    }
    Ok(parsed)
}

pub(crate) fn join_endpoint(root: &Url, path: &str) -> Result<Url> {
    root.join(path).map_err(|err| WepubError::Url {
        url: path.to_string(),
        source: err,
    })
}

pub(crate) async fn send_request(
    client: &reqwest::Client,
    req: reqwest::Request,
) -> Result<reqwest::Response> {
    tracing::debug!(
        method = req.method().as_str(),
        url = req.url().as_str(),
        "sending request"
    );
    let resp = client.execute(req).await?;
    Ok(resp)
}

pub(crate) async fn decode_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T> {
    let status = resp.status();
    let body = resp.text().await?;
    tracing::debug!(
        status = status.as_u16(),
        body = body.as_str(),
        "received response"
    );
    if !status.is_success() {
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    serde_json::from_str(&body).map_err(|err| WepubError::UnexpectedResponse {
        reason: format!("failed to decode response: {err}"),
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
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    #[test]
    fn join_endpoint_appends_relative_path() {
        let root = Url::parse("https://example.com/").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/v5/addons/upload/");
    }
}
