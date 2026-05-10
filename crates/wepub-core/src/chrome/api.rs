use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Result, WepubError, http::build_client};

use super::auth::{TOKEN_URL, refresh_access_token};

const DEFAULT_ROOT_URL: &str = "https://chromewebstore.googleapis.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone)]
enum Credentials {
    AccessToken(String),
    ClientCredentials {
        client_id: String,
        client_secret: String,
        refresh_token: String,
    },
}

pub struct ChromeStore {
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    root_url: Url,
    token_url: Url,
    client: reqwest::Client,
}

impl ChromeStore {
    #[must_use]
    pub fn from_access_token(publisher_id: String, item_id: String, access_token: String) -> Self {
        Self::with_credentials(
            publisher_id,
            item_id,
            Credentials::AccessToken(access_token),
        )
        .expect("default URLs are valid and HTTP client builds")
    }

    #[must_use]
    pub fn from_client_credentials(
        publisher_id: String,
        item_id: String,
        client_id: String,
        client_secret: String,
        refresh_token: String,
    ) -> Self {
        Self::with_credentials(
            publisher_id,
            item_id,
            Credentials::ClientCredentials {
                client_id,
                client_secret,
                refresh_token,
            },
        )
        .expect("default URLs are valid and HTTP client builds")
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
            token_url: Url::parse(TOKEN_URL).expect("TOKEN_URL is a valid URL"),
            client: build_client()?,
        })
    }

    #[must_use]
    pub fn with_base_urls(mut self, root_url: Url, token_url: Url) -> Self {
        self.root_url = ensure_trailing_slash(root_url);
        self.token_url = token_url;
        self
    }

    pub(crate) fn endpoint(&self, path: &str) -> Result<Url> {
        self.root_url
            .join(path)
            .map_err(|e| WepubError::Auth(format!("invalid endpoint path {path:?}: {e}")))
    }

    pub(crate) async fn get_token(&self) -> Result<String> {
        match &self.credentials {
            Credentials::AccessToken(token) => Ok(token.clone()),
            Credentials::ClientCredentials {
                client_id,
                client_secret,
                refresh_token,
            } => {
                refresh_access_token(
                    &self.client,
                    self.token_url.as_str(),
                    client_id,
                    client_secret,
                    refresh_token,
                )
                .await
            }
        }
    }

    pub(crate) async fn upload(&self, token: &str, zip: Vec<u8>) -> Result<UploadState> {
        let url = self.endpoint(&format!(
            "upload/v2/publishers/{}/items/{}:upload",
            self.publisher_id, self.item_id
        ))?;

        tracing::info!(
            publisher_id = %self.publisher_id,
            item_id = %self.item_id,
            "uploading extension to Chrome Web Store"
        );

        let resp = self
            .client
            .post(url)
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(zip)
            .send()
            .await?;

        let body: UploadResponse = decode_response(resp).await?;
        Ok(body.upload_state)
    }

    pub(crate) async fn fetch_status(&self, token: &str) -> Result<Option<UploadState>> {
        let url = self.endpoint(&format!(
            "v2/publishers/{}/items/{}:fetchStatus",
            self.publisher_id, self.item_id
        ))?;

        let resp = self.client.get(url).bearer_auth(token).send().await?;

        let body: FetchStatusResponse = decode_response(resp).await?;
        Ok(body.last_async_upload_state)
    }

    pub(crate) async fn wait_until_uploaded(
        &self,
        token: &str,
        initial_state: UploadState,
        config: &PollConfig,
    ) -> Result<UploadState> {
        let started = Instant::now();
        // First iteration uses the caller-provided state from the initial
        // upload response; subsequent iterations re-fetch from the server.
        let mut state: Option<UploadState> = Some(initial_state);

        loop {
            tracing::info!(
                publisher_id = %self.publisher_id,
                item_id = %self.item_id,
                state = ?state,
                "polling Chrome Web Store upload status"
            );

            match state {
                Some(UploadState::Succeeded) => return Ok(UploadState::Succeeded),
                Some(UploadState::Failed) => {
                    return Err(WepubError::UploadFailed {
                        item_id: self.item_id.clone(),
                        body: "The upload failed.".to_string(),
                    });
                }
                // None (lastAsyncUploadState absent, i.e. no async upload in
                // the past 24h) is treated the same as the explicit NOT_FOUND
                // value: we have just uploaded, so the server should know
                // about it. Either response indicates something is wrong.
                Some(UploadState::NotFound) | None => {
                    return Err(WepubError::UploadFailed {
                        item_id: self.item_id.clone(),
                        body: "An upload attempt was not found.".to_string(),
                    });
                }
                Some(UploadState::InProgress) => {}
            }

            if started.elapsed() >= config.timeout {
                return Err(WepubError::UploadFailed {
                    item_id: self.item_id.clone(),
                    body: format!("upload polling timed out after {:?}", config.timeout),
                });
            }

            tokio::time::sleep(config.interval).await;
            state = self.fetch_status(token).await?;
        }
    }

    pub(crate) async fn submit_for_publish(
        &self,
        token: &str,
        options: &PublishOptions,
    ) -> Result<PublishResponse> {
        let url = self.endpoint(&format!(
            "v2/publishers/{}/items/{}:publish",
            self.publisher_id, self.item_id
        ))?;

        let body = PublishRequestBody::from(options);

        tracing::info!(
            publisher_id = %self.publisher_id,
            item_id = %self.item_id,
            "submitting Chrome Web Store item for publish"
        );

        let resp = self
            .client
            .post(url)
            .bearer_auth(token)
            .json(&body)
            .send()
            .await?;

        decode_response(resp).await
    }

    pub async fn publish(&self, zip: Vec<u8>, options: PublishOptions) -> Result<PublishResponse> {
        let token = self.get_token().await?;
        let initial = self.upload(&token, zip).await?;
        self.wait_until_uploaded(&token, initial, &options.poll)
            .await?;
        self.submit_for_publish(&token, &options).await
    }
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

#[derive(Debug, Default, Serialize)]
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
                PublishType::Default => None,
                PublishType::Staged => Some("STAGED_PUBLISH"),
            },
            skip_review: if opts.skip_review { Some(true) } else { None },
            deploy_infos: opts.deploy_percentage.map(|p| {
                vec![DeployInfo {
                    deploy_percentage: p,
                }]
            }),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DeployInfo {
    deploy_percentage: u8,
}

async fn decode_response<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(WepubError::Api {
            status: status.as_u16(),
            body,
        });
    }
    let body = resp.bytes().await?;
    serde_json::from_slice(&body).map_err(WepubError::from)
}

fn ensure_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let new_path = format!("{}/", url.path());
        url.set_path(&new_path);
    }
    url
}

pub struct PublishOptions {
    pub publish_type: PublishType,
    pub skip_review: bool,
    pub deploy_percentage: Option<u8>,
    pub poll: PollConfig,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PublishType {
    #[default]
    Default,
    Staged,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResponse {
    pub item_id: String,
    pub state: ItemState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ItemState {
    PendingReview,
    Staged,
    Published,
    PublishedToTesters,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum UploadState {
    Succeeded,
    InProgress,
    Failed,
    NotFound,
}

pub struct PollConfig {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_POLL_TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WepubError;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn store_for(server: &MockServer) -> ChromeStore {
        let base = Url::parse(&server.uri()).unwrap();
        ChromeStore::from_access_token(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "test-access-token".to_string(),
        )
        .with_base_urls(base.clone(), base)
    }

    fn fast_poll() -> PollConfig {
        PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        }
    }

    fn default_options() -> PublishOptions {
        PublishOptions {
            publish_type: PublishType::Default,
            skip_review: false,
            deploy_percentage: None,
            poll: fast_poll(),
        }
    }

    const TEST_TOKEN: &str = "test-access-token";

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
            WepubError::Api { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("Unauthorized"));
            }
            other => panic!("expected Api error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn fetch_status_gets_correct_url_with_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/publishers/publisher-1/items/item-1:fetchStatus"))
            .and(header("authorization", "Bearer test-access-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
                "lastAsyncUploadState": "SUCCEEDED",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let state = store.fetch_status(TEST_TOKEN).await.unwrap();
        assert!(matches!(state, Some(UploadState::Succeeded)));
    }

    // Per official docs, lastAsyncUploadState "is only set when there has been an
    // async upload for the item in the past 24 hours". Absent field is distinct
    // from the NOT_FOUND enum value ("an upload attempt was not found"), so we
    // surface absence as None instead of conflating it with NotFound.
    #[tokio::test]
    async fn fetch_status_returns_none_when_last_async_upload_state_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
            })))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let state = store.fetch_status(TEST_TOKEN).await.unwrap();
        assert!(state.is_none());
    }

    #[tokio::test]
    async fn wait_until_uploaded_returns_immediately_when_initial_is_succeeded() {
        let server = MockServer::start().await;
        // Caller already knows the upload succeeded, so wait must not call
        // fetchStatus at all.
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
        // FAILED is terminal, so no fetchStatus call should be made.
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
            WepubError::UploadFailed { item_id, body } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(body, "The upload failed.");
            }
            other => panic!("expected UploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_polls_until_succeeded() {
        let server = MockServer::start().await;
        // Initial state is IN_PROGRESS, so wait must sleep then call
        // fetchStatus, which on this mock returns SUCCEEDED.
        Mock::given(method("GET"))
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

    // The fetchStatus V2 response schema does not include any field for
    // failure detail - only `lastAsyncUploadState`. So when state is FAILED,
    // we surface the official enum description verbatim and stop there.
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
            WepubError::UploadFailed { item_id, body } => {
                assert_eq!(item_id, "item-1");
                assert_eq!(body, "The upload failed.");
            }
            other => panic!("expected UploadFailed, got {other:?}"),
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

    // With all fields at defaults the request body must contain none of the
    // optional keys, so the server side falls back to API defaults.
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
        opts.publish_type = PublishType::Staged;

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
        opts.skip_review = true;
        opts.deploy_percentage = Some(50);

        let store = store_for(&server);
        let resp = store.submit_for_publish(TEST_TOKEN, &opts).await.unwrap();
        assert!(matches!(resp.state, ItemState::Published));
    }

    // The six meaningful ItemState wire values from the official docs must
    // round-trip. ITEM_STATE_UNSPECIFIED is documented as "unused" so we drop
    // it from the public enum and let serde fail-fast if it ever appears.
    #[tokio::test]
    async fn submit_for_publish_decodes_all_item_states() {
        let cases = [
            ("PENDING_REVIEW", ItemState::PendingReview),
            ("STAGED", ItemState::Staged),
            ("PUBLISHED", ItemState::Published),
            ("PUBLISHED_TO_TESTERS", ItemState::PublishedToTesters),
            ("REJECTED", ItemState::Rejected),
            ("CANCELLED", ItemState::Cancelled),
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
        let resp = store
            .publish(b"FAKE_ZIP_BYTES".to_vec(), default_options())
            .await
            .unwrap();
        assert_eq!(resp.item_id, "item-1");
    }

    // When upload returns SUCCEEDED immediately, no fetchStatus call should be
    // made; we go straight from upload to the publish endpoint.
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

    // End-to-end check that from_client_credentials's get_token triggers a
    // token exchange and the resulting access token can be used as a Bearer
    // credential on subsequent API calls.
    #[tokio::test]
    async fn from_client_credentials_refreshes_token_before_calling_api() {
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
        let store = ChromeStore::from_client_credentials(
            "publisher-1".to_string(),
            "item-1".to_string(),
            "client-id".to_string(),
            "client-secret".to_string(),
            "refresh-token".to_string(),
        )
        .with_base_urls(base.clone(), base);

        let token = store.get_token().await.unwrap();
        assert_eq!(token, "fresh-token");
        store.upload(&token, b"FAKE".to_vec()).await.unwrap();
    }
}
