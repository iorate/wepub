use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, log_request, parse_root_url},
    http::build_client,
};

use super::auth::{DEFAULT_TOKEN_URL, refresh_access_token};

const DEFAULT_ROOT_URL: &str = "https://chromewebstore.googleapis.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Options that shape how [`Store::publish`] submits the new version.
#[derive(Debug, Clone)]
pub struct PublishOptions {
    /// Whether the version goes live immediately after review or stays in
    /// staging for a manual rollout from the Developer Dashboard.
    pub publish_type: Option<PublishType>,

    /// Bypass the standard review queue.
    pub skip_review: Option<bool>,

    /// Initial percentage of users to roll the new version out to.
    /// `None` means "use the value configured in the Developer Dashboard".
    pub deploy_percentage: Option<u8>,

    /// Polling cadence and overall timeout used while waiting for the
    /// asynchronous upload to finish processing.
    pub poll: PollConfig,
}

impl PublishOptions {
    /// Build a `PublishOptions` with the recommended defaults
    /// (2 second poll interval, 5 minute timeout).
    #[must_use]
    pub fn new() -> Self {
        Self {
            publish_type: None,
            skip_review: None,
            deploy_percentage: None,
            poll: PollConfig {
                interval: DEFAULT_POLL_INTERVAL,
                timeout: DEFAULT_POLL_TIMEOUT,
            },
        }
    }
}

impl Default for PublishOptions {
    fn default() -> Self {
        Self::new()
    }
}

/// Whether a successfully reviewed version goes live immediately or waits in
/// staging for a manual rollout.
#[derive(Debug, Clone, Copy, Default)]
pub enum PublishType {
    /// Publish immediately after review (the default).
    #[default]
    Default,
    /// Hold the reviewed version in staging until a Developer Dashboard
    /// operator triggers the rollout.
    Staged,
}

/// Client for the Chrome Web Store Publish API (v2).
///
/// The store holds OAuth credentials and a reusable HTTP client; it is cheap
/// to construct and intended to live for the duration of a single publish
/// run.
// Debug intentionally omitted: holds OAuth credentials.
pub struct Store {
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    root_url: Url,
    token_url: Url,
    client: reqwest::Client,
}

impl Store {
    /// Build a store from a pre-fetched OAuth access token.
    ///
    /// Intended for service-account authentication. The token is used
    /// verbatim; this constructor never touches the OAuth token endpoint.
    ///
    /// # Errors
    ///
    /// Fails if the underlying HTTP client cannot be built.
    pub fn from_access_token(
        publisher_id: String,
        item_id: String,
        access_token: String,
    ) -> Result<Self> {
        Self::with_credentials(
            publisher_id,
            item_id,
            Credentials::AccessToken(access_token),
        )
    }

    /// Build a store from a long-lived OAuth refresh token.
    ///
    /// An access token is fetched lazily during [`publish`](Store::publish).
    ///
    /// # Errors
    ///
    /// Fails if the underlying HTTP client cannot be built.
    pub fn from_credentials(
        publisher_id: String,
        item_id: String,
        client_id: String,
        client_secret: String,
        refresh_token: String,
    ) -> Result<Self> {
        Self::with_credentials(
            publisher_id,
            item_id,
            Credentials::ClientCredentials {
                client_id,
                client_secret,
                refresh_token,
            },
        )
    }

    /// Override the Chrome Web Store API root URL.
    ///
    /// Defaults to `https://chromewebstore.googleapis.com/`. Intended for
    /// tests that point the client at a mock server. A missing trailing
    /// slash is added automatically so that relative paths join correctly.
    ///
    /// # Errors
    ///
    /// Returns [`WepubError::InvalidUrl`] if `root_url` does not parse as a
    /// URL.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Override the Google OAuth token endpoint URL.
    ///
    /// Defaults to `https://oauth2.googleapis.com/token`. Intended for
    /// tests; only consulted when the store was built with
    /// [`from_credentials`](Store::from_credentials).
    ///
    /// # Errors
    ///
    /// Returns [`WepubError::InvalidUrl`] if `token_url` does not parse as a
    /// URL.
    pub fn with_token_url(mut self, token_url: &str) -> Result<Self> {
        self.token_url = Url::parse(token_url)
            .map_err(|e| WepubError::InvalidUrl(format!("{token_url:?}: {e}")))?;
        Ok(self)
    }

    /// Upload `zip` and submit the resulting item version for publish.
    ///
    /// If the upload is still in progress when it is accepted, the call
    /// polls for completion according to `options.poll` until the upload
    /// succeeds or the timeout elapses. A publish request that reaches a
    /// terminal failure state is reported as
    /// [`WepubError::ChromePublishFailed`].
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> wepub_core::Result<()> {
    /// use wepub_core::chrome::{PublishOptions, PublishType, Store};
    ///
    /// let store = Store::from_credentials(
    ///     "publisher-1".into(),
    ///     "abcdefghijklmnopabcdefghijklmnop".into(),
    ///     "client-id".into(),
    ///     "client-secret".into(),
    ///     "refresh-token".into(),
    /// )?;
    /// let zip = std::fs::read("./extension.zip")?;
    /// store
    ///     .publish(
    ///         zip,
    ///         PublishOptions {
    ///             publish_type: Some(PublishType::Staged),
    ///             ..PublishOptions::new()
    ///         },
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(&self, zip: Vec<u8>, options: PublishOptions) -> Result<()> {
        let token = self.get_token().await?;
        let initial = self.upload(&token, zip).await?;
        self.wait_until_uploaded(&token, initial, &options.poll)
            .await?;
        self.submit_for_publish(&token, &options).await?;
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    async fn get_token(&self) -> Result<String> {
        match &self.credentials {
            Credentials::AccessToken(token) => Ok(token.clone()),
            Credentials::ClientCredentials {
                client_id,
                client_secret,
                refresh_token,
            } => {
                refresh_access_token(
                    &self.client,
                    &self.token_url,
                    client_id,
                    client_secret,
                    refresh_token,
                )
                .await
            }
        }
    }

    async fn upload(&self, token: &str, zip: Vec<u8>) -> Result<UploadState> {
        tracing::info!(
            publisher_id = %self.publisher_id,
            item_id = %self.item_id,
            "uploading"
        );

        let method = reqwest::Method::POST;
        let url = self.endpoint(&format!(
            "upload/v2/publishers/{}/items/{}:upload",
            self.publisher_id, self.item_id
        ))?;

        log_request(&method, &url);
        let resp = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(zip)
            .send()
            .await?;

        let body: UploadResponse = decode_response(resp).await?;
        Ok(body.upload_state)
    }

    async fn wait_until_uploaded(
        &self,
        token: &str,
        initial_state: UploadState,
        config: &PollConfig,
    ) -> Result<UploadState> {
        let url = self.endpoint(&format!(
            "v2/publishers/{}/items/{}:fetchStatus",
            self.publisher_id, self.item_id
        ))?;
        let started = Instant::now();
        // First iteration uses the caller-provided state from the initial
        // upload response; subsequent iterations re-fetch from the server.
        let mut initial = true;

        loop {
            let state: Option<UploadState> = if initial {
                initial = false;
                Some(initial_state)
            } else {
                tracing::info!(
                    publisher_id = %self.publisher_id,
                    item_id = %self.item_id,
                    "polling upload status"
                );
                let method = reqwest::Method::GET;
                log_request(&method, &url);
                let resp = self
                    .client
                    .request(method, url.clone())
                    .bearer_auth(token)
                    .send()
                    .await?;
                let body: FetchStatusResponse = decode_response(resp).await?;
                body.last_async_upload_state
            };

            let reason = match state {
                Some(UploadState::Succeeded) => return Ok(UploadState::Succeeded),
                Some(UploadState::InProgress) => None,
                Some(UploadState::Failed) => Some("upload failed"),
                // Absent `lastAsyncUploadState` is documented as "no async
                // upload in the past 24 hours" - same as NOT_FOUND for us.
                Some(UploadState::NotFound) | None => Some("upload not found"),
            };
            if let Some(reason) = reason {
                return Err(WepubError::ChromeUploadFailed {
                    item_id: self.item_id.clone(),
                    reason: reason.to_string(),
                });
            }

            let elapsed = started.elapsed();
            if elapsed >= config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    async fn submit_for_publish(
        &self,
        token: &str,
        options: &PublishOptions,
    ) -> Result<PublishResponse> {
        tracing::info!(
            publisher_id = %self.publisher_id,
            item_id = %self.item_id,
            "submitting for publish"
        );

        let method = reqwest::Method::POST;
        let url = self.endpoint(&format!(
            "v2/publishers/{}/items/{}:publish",
            self.publisher_id, self.item_id
        ))?;

        let body = PublishRequestBody::from(options);

        log_request(&method, &url);
        let resp = self
            .client
            .request(method, url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;

        let parsed: PublishResponse = decode_response(resp).await?;
        let reason = match parsed.state {
            ItemState::PendingReview
            | ItemState::Staged
            | ItemState::Published
            | ItemState::PublishedToTesters => None,
            ItemState::Rejected => Some("rejected"),
            ItemState::Cancelled => Some("cancelled"),
        };
        if let Some(reason) = reason {
            return Err(WepubError::ChromePublishFailed {
                item_id: parsed.item_id,
                reason: reason.to_string(),
            });
        }
        tracing::info!(
            item_id = %parsed.item_id,
            state = ?parsed.state,
            "publish succeeded"
        );
        Ok(parsed)
    }

    fn with_credentials(
        publisher_id: String,
        item_id: String,
        credentials: Credentials,
    ) -> Result<Self> {
        Ok(Self {
            publisher_id,
            item_id,
            credentials,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            token_url: Url::parse(DEFAULT_TOKEN_URL).expect("DEFAULT_TOKEN_URL is a valid URL"),
            client: build_client()?,
        })
    }
}

// Debug and Clone intentionally omitted: holds OAuth secrets / refresh token.
enum Credentials {
    AccessToken(String),
    ClientCredentials {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    upload_state: UploadState,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchStatusResponse {
    #[serde(default)]
    last_async_upload_state: Option<UploadState>,
}

// `UPLOAD_STATE_UNSPECIFIED` is documented as unused, so serde will reject it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum UploadState {
    Succeeded,
    InProgress,
    Failed,
    NotFound,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishRequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    publish_type: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_review: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deploy_infos: Option<Vec<DeployInfo>>,
}

impl From<&PublishOptions> for PublishRequestBody {
    fn from(opts: &PublishOptions) -> Self {
        Self {
            publish_type: match opts.publish_type {
                Some(PublishType::Default) => Some("DEFAULT_PUBLISH"),
                Some(PublishType::Staged) => Some("STAGED_PUBLISH"),
                None => None,
            },
            skip_review: opts.skip_review,
            deploy_infos: opts.deploy_percentage.map(|p| {
                vec![DeployInfo {
                    deploy_percentage: p,
                }]
            }),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeployInfo {
    deploy_percentage: u8,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishResponse {
    item_id: String,
    state: ItemState,
}

// `ITEM_STATE_UNSPECIFIED` is documented as unused, so serde will reject it.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ItemState {
    PendingReview,
    Staged,
    Published,
    PublishedToTesters,
    Rejected,
    Cancelled,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WepubError;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const TEST_TOKEN: &str = "test-access-token";

    #[tokio::test]
    async fn from_credentials_refreshes_token_before_calling_api() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "access_token": "fresh-token" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(
                "/upload/v2/publishers/publisher-1/items/item-1:upload",
            ))
            .and(header("authorization", "Bearer fresh-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let base = Url::parse(&server.uri()).unwrap();
        let store = Store::from_credentials(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "client-id".to_string(),
            "client-secret".to_string(),
            "refresh-token".to_string(),
        )
        .unwrap()
        .with_root_url(base.as_str())
        .unwrap()
        .with_token_url(base.as_str())
        .unwrap();

        let token = store.get_token().await.unwrap();
        assert_eq!(token, "fresh-token");
        store.upload(&token, b"FAKE".to_vec()).await.unwrap();
    }

    #[tokio::test]
    async fn upload_posts_to_correct_url_with_auth_and_octet_stream() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/upload/v2/publishers/publisher-1/items/item-1:upload",
            ))
            .and(header("authorization", "Bearer test-access-token"))
            .and(header("content-type", "application/octet-stream"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
                "uploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        store
            .upload(TEST_TOKEN, b"FAKE_ZIP_BYTES".to_vec())
            .await
            .unwrap();
    }

    // Regression guard: official V2 curl example sends neither X-Goog-Upload-Protocol
    // nor X-Goog-Upload-File-Name. fregante does, but we follow the official example.
    #[tokio::test]
    async fn upload_does_not_send_x_goog_upload_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/upload/v2/publishers/publisher-1/items/item-1:upload",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "SUCCEEDED",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        store
            .upload(TEST_TOKEN, b"FAKE_ZIP_BYTES".to_vec())
            .await
            .unwrap();

        for req in server.received_requests().await.unwrap_or_default() {
            assert!(
                req.headers.get("x-goog-upload-protocol").is_none(),
                "must not send X-Goog-Upload-Protocol",
            );
            assert!(
                req.headers.get("x-goog-upload-file-name").is_none(),
                "must not send X-Goog-Upload-File-Name",
            );
        }
    }

    #[tokio::test]
    async fn upload_returns_upload_state_from_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "IN_PROGRESS",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let resp = store.upload(TEST_TOKEN, b"FAKE".to_vec()).await.unwrap();
        assert!(matches!(resp, UploadState::InProgress));
    }

    #[tokio::test]
    async fn upload_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .upload(TEST_TOKEN, b"FAKE".to_vec())
            .await
            .unwrap_err();
        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("Unauthorized"));
            }
            other => panic!("expected HttpStatus error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_returns_immediately_when_initial_is_succeeded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "SUCCEEDED",
            })))
            .expect(0)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let state = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::Succeeded, &fast_poll())
            .await
            .unwrap();
        assert!(matches!(state, UploadState::Succeeded));
    }

    #[tokio::test]
    async fn wait_until_uploaded_errors_immediately_when_initial_is_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::Failed, &fast_poll())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromeUploadFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "upload failed");
            }
            other => panic!("expected WepubError::ChromeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_polls_until_succeeded() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/publishers/publisher-1/items/item-1:fetchStatus"))
            .and(header("authorization", "Bearer test-access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let state = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::InProgress, &fast_poll())
            .await
            .unwrap();
        assert!(matches!(state, UploadState::Succeeded));
    }

    #[tokio::test]
    async fn wait_until_uploaded_errors_when_polling_returns_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "FAILED",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::InProgress, &fast_poll())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromeUploadFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "upload failed");
            }
            other => panic!("expected WepubError::ChromeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_errors_when_polling_response_omits_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::InProgress, &fast_poll())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromeUploadFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "upload not found");
            }
            other => panic!("expected WepubError::ChromeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_errors_when_polling_returns_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "NOT_FOUND",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::InProgress, &fast_poll())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromeUploadFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "upload not found");
            }
            other => panic!("expected WepubError::ChromeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "IN_PROGRESS",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_uploaded(TEST_TOKEN, UploadState::InProgress, &fast_poll())
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("timeout") || msg.to_lowercase().contains("timed out"),
            "expected timeout error, got: {msg}",
        );
    }

    #[tokio::test]
    async fn submit_for_publish_default_sends_minimal_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .and(header("authorization", "Bearer test-access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
                "state": "PENDING_REVIEW",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let resp = store
            .submit_for_publish(TEST_TOKEN, &default_options())
            .await
            .unwrap();
        assert_eq!(resp.item_id, "item-1");
        assert!(matches!(resp.state, ItemState::PendingReview));

        let received = server.received_requests().await.unwrap();
        let body_str = std::str::from_utf8(&received[0].body).unwrap();
        assert!(
            !body_str.contains("publishType"),
            "Default must not send publishType key. body: {body_str}",
        );
        assert!(
            !body_str.contains("skipReview"),
            "false skip_review must not send skipReview key. body: {body_str}",
        );
        assert!(
            !body_str.contains("deployInfos"),
            "None deploy_percentage must not send deployInfos key. body: {body_str}",
        );
    }

    #[tokio::test]
    async fn submit_for_publish_staged_sends_publish_type() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .and(body_string_contains("\"publishType\":\"STAGED_PUBLISH\""))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "STAGED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut opts = default_options();
        opts.publish_type = Some(PublishType::Staged);

        let store = store_for(&server);
        let resp = store.submit_for_publish(TEST_TOKEN, &opts).await.unwrap();
        assert!(matches!(resp.state, ItemState::Staged));
    }

    #[tokio::test]
    async fn submit_for_publish_with_skip_review_and_deploy_percentage() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(body_string_contains("\"skipReview\":true"))
            .and(body_string_contains("\"deployInfos\""))
            .and(body_string_contains("\"deployPercentage\":50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "PUBLISHED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let mut opts = default_options();
        opts.skip_review = Some(true);
        opts.deploy_percentage = Some(50);

        let store = store_for(&server);
        let resp = store.submit_for_publish(TEST_TOKEN, &opts).await.unwrap();
        assert!(matches!(resp.state, ItemState::Published));
    }

    #[tokio::test]
    async fn submit_for_publish_errors_on_rejected_state() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "REJECTED",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .submit_for_publish(TEST_TOKEN, &default_options())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromePublishFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "rejected");
            }
            other => panic!("expected WepubError::ChromePublishFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_for_publish_errors_on_cancelled_state() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "CANCELLED",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .submit_for_publish(TEST_TOKEN, &default_options())
            .await
            .unwrap_err();
        match err {
            WepubError::ChromePublishFailed { item_id, reason } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(reason, "cancelled");
            }
            other => panic!("expected WepubError::ChromePublishFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_for_publish_decodes_non_terminal_item_states() {
        let cases = [
            ("PENDING_REVIEW", ItemState::PendingReview),
            ("STAGED", ItemState::Staged),
            ("PUBLISHED", ItemState::Published),
            ("PUBLISHED_TO_TESTERS", ItemState::PublishedToTesters),
        ];

        for (wire, expected) in cases {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "itemId": "item-1",
                    "state": wire,
                })))
                .mount(&server)
                .await;

            let store = store_for(&server);
            let resp = store
                .submit_for_publish(TEST_TOKEN, &default_options())
                .await
                .unwrap();
            assert!(
                std::mem::discriminant(&resp.state) == std::mem::discriminant(&expected),
                "wire value {wire} should decode to expected variant",
            );
        }
    }

    #[tokio::test]
    async fn publish_full_happy_path() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path(
                "/upload/v2/publishers/publisher-1/items/item-1:upload",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "IN_PROGRESS",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/v2/publishers/publisher-1/items/item-1:fetchStatus"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "PENDING_REVIEW",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        store
            .publish(b"FAKE_ZIP_BYTES".to_vec(), default_options())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_skips_polling_when_upload_returns_succeeded() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path(
                "/upload/v2/publishers/publisher-1/items/item-1:upload",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "SUCCEEDED",
            })))
            .expect(0)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "PUBLISHED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        store
            .publish(b"FAKE_ZIP_BYTES".to_vec(), default_options())
            .await
            .unwrap();
    }

    #[test]
    fn with_root_url_rejects_garbage() {
        let store = Store::from_access_token(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "token".to_string(),
        )
        .unwrap();
        let Err(err) = store.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    #[test]
    fn with_token_url_rejects_garbage() {
        let store = Store::from_access_token(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "token".to_string(),
        )
        .unwrap();
        let Err(err) = store.with_token_url("not a url") else {
            panic!("expected with_token_url to reject");
        };
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    fn store_for(server: &MockServer) -> Store {
        let base = server.uri();
        Store::from_access_token(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "test-access-token".to_string(),
        )
        .unwrap()
        .with_root_url(&base)
        .unwrap()
        .with_token_url(&base)
        .unwrap()
    }

    fn fast_poll() -> PollConfig {
        PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        }
    }

    fn default_options() -> PublishOptions {
        PublishOptions {
            publish_type: None,
            skip_review: None,
            deploy_percentage: None,
            poll: fast_poll(),
        }
    }
}
