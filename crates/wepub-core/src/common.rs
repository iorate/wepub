use tracing::{Instrument, Level, Span, debug};
use url::Url;

use crate::{Result, WepubError, error::record_error};

pub(crate) async fn instrument_step<T>(
    span: Span,
    error_level: Level,
    fut: impl std::future::Future<Output = Result<T>>,
) -> Result<T> {
    async move { fut.await.inspect_err(|err| record_error(error_level, err)) }
        .instrument(span)
        .await
}

// A trailing slash makes `Url::join` append rather than replace the last segment.
pub(crate) fn ensure_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let new_path = format!("{}/", url.path());
        url.set_path(&new_path);
    }
    url
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
    debug!(
        method = req.method().as_str(),
        url = req.url().as_str(),
        "sending request"
    );
    let resp = client.execute(req).await.map_err(WepubError::http)?;
    Ok(resp)
}

pub(crate) async fn decode_response<T: serde::de::DeserializeOwned>(
    resp: reqwest::Response,
) -> Result<T> {
    let status = resp.status();
    let body = resp.text().await.map_err(WepubError::http)?;
    debug!(
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
    fn ensure_trailing_slash_appends_when_missing() {
        let url = ensure_trailing_slash(Url::parse("https://example.com/api/v5").unwrap());
        assert_eq!(url.as_str(), "https://example.com/api/v5/");
    }

    #[test]
    fn ensure_trailing_slash_preserves_existing() {
        let url = ensure_trailing_slash(Url::parse("https://example.com/api/v5/").unwrap());
        assert_eq!(url.as_str(), "https://example.com/api/v5/");
    }

    #[test]
    fn join_endpoint_appends_relative_path() {
        let root = Url::parse("https://example.com/").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "https://example.com/api/v5/addons/upload/");
    }
}
