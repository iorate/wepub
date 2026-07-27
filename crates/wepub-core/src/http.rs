use std::time::Duration;

use isahc::config::{Configurable, RedirectPolicy};
use isahc::http::{Request, Response, header};
use isahc::{AsyncBody, AsyncReadResponseExt, HttpClient};
use tracing::debug;
use url::Url;

use crate::{Result, WepubError};

const USER_AGENT: &str = concat!("wepub/", env!("CARGO_PKG_VERSION"));
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_SPEED_LIMIT: u32 = 1; // bytes/sec
const STALL_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) fn build_client() -> Result<HttpClient> {
    let client = HttpClient::builder()
        .default_header(header::USER_AGENT, USER_AGENT)
        .connect_timeout(CONNECT_TIMEOUT)
        .low_speed_timeout(STALL_SPEED_LIMIT, STALL_TIMEOUT)
        .expect_continue(false)
        .redirect_policy(RedirectPolicy::Limit(10))
        .build()
        .map_err(WepubError::http)?;
    Ok(client)
}

pub(crate) fn join_endpoint(root: &Url, path: &str) -> Url {
    let mut url = root.clone();
    url.set_path(&format!("{}/{}", root.path().trim_end_matches('/'), path));
    url
}

pub(crate) async fn send_request<B: Into<AsyncBody>>(
    client: &HttpClient,
    req: Request<B>,
) -> Result<Response<AsyncBody>> {
    let url = req.uri().to_string();
    debug!(
        method = req.method().as_str(),
        url = url.as_str(),
        "sending request"
    );
    let resp = client.send_async(req).await.map_err(WepubError::http)?;
    Ok(resp)
}

pub(crate) async fn decode_response<T: serde::de::DeserializeOwned>(
    mut resp: Response<AsyncBody>,
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
