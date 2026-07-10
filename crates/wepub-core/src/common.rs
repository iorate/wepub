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

pub(crate) fn join_endpoint(root: &Url, path: &str) -> Url {
    let mut url = root.clone();
    url.set_path(&format!("{}/{}", root.path().trim_end_matches('/'), path));
    url
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
    fn join_endpoint_appends_relative_path() {
        let root = Url::parse("https://example.com/").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/");
        assert_eq!(url.as_str(), "https://example.com/api/v5/addons/upload/");
    }

    #[test]
    fn join_endpoint_appends_to_root_with_path() {
        let root = Url::parse("https://example.com/prefix/").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/");
        assert_eq!(
            url.as_str(),
            "https://example.com/prefix/api/v5/addons/upload/"
        );
    }

    #[test]
    fn join_endpoint_appends_to_root_without_trailing_slash() {
        let root = Url::parse("https://example.com/prefix").unwrap();
        let url = join_endpoint(&root, "api/v5/addons/upload/");
        assert_eq!(
            url.as_str(),
            "https://example.com/prefix/api/v5/addons/upload/"
        );
    }
}
