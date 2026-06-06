use std::borrow::Cow;
use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, parse_root_url, send_request},
    http::build_client,
};

use super::auth::{self, DEFAULT_TOKEN_URL};

const DEFAULT_ROOT_URL: &str = "https://chromewebstore.googleapis.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// OAuth credentials passed to [`Client::new`].
#[derive(Clone)]
pub enum Credentials {
    /// An OAuth refresh token plus the client credentials needed to redeem
    /// it for an access token. Obtain them by following
    /// [Use the Chrome Web Store API](https://developer.chrome.com/docs/webstore/using-api).
    RefreshToken {
        /// OAuth client ID.
        client_id: String,
        /// OAuth client secret.
        client_secret: String,
        /// OAuth refresh token.
        refresh_token: String,
    },
    /// A pre-fetched OAuth access token, used verbatim. Suitable for
    /// automated workflows that authenticate with a
    /// [service account](https://developer.chrome.com/docs/webstore/service-accounts).
    AccessToken(String),
}

// Hand-written so secrets never reach `Debug` output.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RefreshToken { .. } => f.debug_struct("RefreshToken").finish_non_exhaustive(),
            Self::AccessToken(_) => f.debug_tuple("AccessToken").finish_non_exhaustive(),
        }
    }
}

/// Options that shape how [`Client::publish`] submits the new version.
#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    /// Whether to publish immediately on approval or stage for later
    /// publishing.
    pub publish_type: Option<PublishType>,

    /// Initial percentage of users to roll the new version out to.
    /// `None` means "use the value configured in the Developer Dashboard".
    pub deploy_percentage: Option<u8>,

    /// Attempt to skip item review.
    pub skip_review: Option<bool>,
}

impl PublishOptions {
    /// Build a `PublishOptions` with all fields unset.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Whether a new version is published immediately on approval or staged for
/// later publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PublishType {
    /// Publish immediately on approval.
    DefaultPublish,
    /// Stage for later publishing.
    StagedPublish,
}

/// Progress events reported by [`Client::publish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Progress {
    /// Refreshing the access token.
    RefreshAccessToken,
    /// Uploading the package archive.
    Upload,
    /// Waiting for the upload to be processed.
    AwaitUpload,
    /// Submitting the draft.
    Submit,
}

/// Client for the Chrome Web Store API (v2).
#[derive(Debug, Clone)]
pub struct Client {
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    root_url: Url,
    token_url: Url,
    poll_config: PollConfig,
    http: reqwest::Client,
}

impl Client {
    /// Build a client bound to `publisher_id` / `item_id`, authenticating
    /// with the supplied `credentials`.
    pub fn new(publisher_id: String, item_id: String, credentials: Credentials) -> Result<Self> {
        Ok(Self {
            publisher_id,
            item_id,
            credentials,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            token_url: Url::parse(DEFAULT_TOKEN_URL).expect("DEFAULT_TOKEN_URL is a valid URL"),
            poll_config: PollConfig {
                interval: DEFAULT_POLL_INTERVAL,
                timeout: DEFAULT_POLL_TIMEOUT,
            },
            http: build_client()?,
        })
    }

    /// Override the Chrome Web Store API root URL.
    ///
    /// Defaults to `https://chromewebstore.googleapis.com/`.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Override the Google OAuth token endpoint URL.
    ///
    /// Defaults to `https://oauth2.googleapis.com/token`.
    pub fn with_token_url(mut self, token_url: &str) -> Result<Self> {
        self.token_url = Url::parse(token_url).map_err(|e| WepubError::Url {
            url: token_url.to_string(),
            source: e,
        })?;
        Ok(self)
    }

    /// Override the poll config.
    #[must_use]
    pub fn with_poll_config(mut self, poll_config: PollConfig) -> Self {
        self.poll_config = poll_config;
        self
    }

    /// Upload `zip` and submit the draft.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use wepub_core::chrome::{Client, Credentials, PublishOptions, PublishType};
    ///
    /// let client = Client::new(
    ///     "abcd1234-ef56-7890-abcd-ef1234567890".into(),
    ///     "abcdefghijklmnopabcdefghijklmnop".into(),
    ///     Credentials::RefreshToken {
    ///         client_id: "client-id".into(),
    ///         client_secret: "client-secret".into(),
    ///         refresh_token: "refresh-token".into(),
    ///     },
    /// )?;
    /// let zip = std::fs::read("./extension.zip")?;
    /// client
    ///     .publish(
    ///         zip,
    ///         PublishOptions {
    ///             publish_type: Some(PublishType::StagedPublish),
    ///             ..PublishOptions::new()
    ///         },
    ///         |_progress| {},
    ///     )
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    #[tracing::instrument(
        skip_all,
        fields(
            store = "chrome",
            publisher_id = self.publisher_id.as_str(),
            item_id = self.item_id.as_str(),
        )
    )]
    pub async fn publish(
        &self,
        zip: Vec<u8>,
        options: PublishOptions,
        on_progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<()> {
        let on_progress = &on_progress as &(dyn Fn(Progress) + Send + Sync);

        let token: Cow<'_, str> = match &self.credentials {
            Credentials::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
            } => {
                let token = self
                    .refresh_access_token(client_id, client_secret, refresh_token, on_progress)
                    .await?;
                Cow::Owned(token)
            }
            Credentials::AccessToken(token) => Cow::Borrowed(token),
        };

        if !self.upload(&token, zip, on_progress).await? {
            self.await_upload(&token, on_progress).await?;
        }

        self.submit(&token, &options, on_progress).await?;

        Ok(())
    }

    #[tracing::instrument(skip_all, err)]
    async fn refresh_access_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<String> {
        tracing::info!("refreshing the access token");
        on_progress(Progress::RefreshAccessToken);

        let token = auth::refresh_access_token(
            &self.http,
            self.token_url.clone(),
            client_id,
            client_secret,
            refresh_token,
        )
        .await?;

        tracing::info!("the access token refreshed");
        Ok(token)
    }

    #[tracing::instrument(skip_all, err)]
    async fn upload(
        &self,
        token: &str,
        zip: Vec<u8>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<bool> {
        tracing::info!("uploading the package archive");
        on_progress(Progress::Upload);

        let req = self
            .http
            .post(self.endpoint(&format!(
                "upload/v2/publishers/{}/items/{}:upload",
                self.publisher_id, self.item_id
            ))?)
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(zip)
            .build()?;

        let resp = send_request(&self.http, req).await?;

        let upload: UploadResponse = decode_response(resp).await?;
        let processed = upload_processed(Some(upload.upload_state))?;

        tracing::info!(
            upload_state = upload.upload_state.as_str(),
            "the package archive uploaded",
        );
        Ok(processed)
    }

    #[tracing::instrument(skip_all, err)]
    async fn await_upload(
        &self,
        token: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        tracing::info!("waiting for the upload to be processed");
        on_progress(Progress::AwaitUpload);

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_config.interval).await;

            let req = self
                .http
                .get(self.endpoint(&format!(
                    "v2/publishers/{}/items/{}:fetchStatus",
                    self.publisher_id, self.item_id
                ))?)
                .bearer_auth(token)
                .build()?;

            let resp = send_request(&self.http, req).await?;

            let status: FetchStatusResponse = decode_response(resp).await?;
            let processed = upload_processed(status.last_async_upload_state)?;
            if processed {
                break;
            }
        }

        tracing::info!("the upload processed");
        Ok(())
    }

    #[tracing::instrument(skip_all, err)]
    async fn submit(
        &self,
        token: &str,
        options: &PublishOptions,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        tracing::info!("submitting the draft");
        on_progress(Progress::Submit);

        let body = PublishRequestBody {
            publish_type: options.publish_type,
            deploy_infos: options.deploy_percentage.map(|p| {
                vec![DeployInfo {
                    deploy_percentage: p,
                }]
            }),
            skip_review: options.skip_review,
        };
        let req = self
            .http
            .post(self.endpoint(&format!(
                "v2/publishers/{}/items/{}:publish",
                self.publisher_id, self.item_id
            ))?)
            .bearer_auth(token)
            .json(&body)
            .build()?;

        let resp = send_request(&self.http, req).await?;

        let publish: PublishResponse = decode_response(resp).await?;
        if let ItemState::Rejected | ItemState::Cancelled = publish.state {
            return Err(WepubError::ChromePublish {
                item_state: publish.state.as_str().to_string(),
            });
        }

        tracing::info!(item_state = publish.state.as_str(), "the draft submitted");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct UploadResponse {
    upload_state: UploadState,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct FetchStatusResponse {
    #[serde(default)]
    last_async_upload_state: Option<UploadState>,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum UploadState {
    Succeeded,
    InProgress,
    Failed,
    NotFound,
}

impl UploadState {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "SUCCEEDED",
            Self::InProgress => "IN_PROGRESS",
            Self::Failed => "FAILED",
            Self::NotFound => "NOT_FOUND",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PublishRequestBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    publish_type: Option<PublishType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    deploy_infos: Option<Vec<DeployInfo>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_review: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeployInfo {
    deploy_percentage: u8,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PublishResponse {
    state: ItemState,
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
enum ItemState {
    PendingReview,
    Staged,
    Published,
    PublishedToTesters,
    Rejected,
    Cancelled,
}

impl ItemState {
    fn as_str(self) -> &'static str {
        match self {
            Self::PendingReview => "PENDING_REVIEW",
            Self::Staged => "STAGED",
            Self::Published => "PUBLISHED",
            Self::PublishedToTesters => "PUBLISHED_TO_TESTERS",
            Self::Rejected => "REJECTED",
            Self::Cancelled => "CANCELLED",
        }
    }
}

fn upload_processed(state: Option<UploadState>) -> Result<bool> {
    match state {
        Some(UploadState::Succeeded) => Ok(true),
        Some(UploadState::InProgress) => Ok(false),
        Some(state) => Err(WepubError::ChromeUpload {
            upload_state: state.as_str().to_string(),
        }),
        None => Err(WepubError::ChromeUpload {
            upload_state: "UPLOAD_STATE_UNSPECIFIED".to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WepubError;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const CLIENT_ID: &str = "client-id";
    const CLIENT_SECRET: &str = "client-secret";
    const REFRESH_TOKEN: &str = "refresh-token";
    const TEST_TOKEN: &str = "test-access-token";

    #[test]
    fn debug_redacts_secrets() {
        let credentials = Credentials::RefreshToken {
            client_id: CLIENT_ID.to_string(),
            client_secret: CLIENT_SECRET.to_string(),
            refresh_token: REFRESH_TOKEN.to_string(),
        };
        let credentials_debug = format!("{credentials:?}");
        assert!(!credentials_debug.contains("secret-client"));
        assert!(!credentials_debug.contains("secret-refresh"));

        let access = Credentials::AccessToken("secret-token".to_string());
        assert!(!format!("{access:?}").contains("secret-token"));

        let client =
            Client::new("publisher-1".to_string(), "item-1".to_string(), credentials).unwrap();
        let client_debug = format!("{client:?}");
        assert!(!client_debug.contains("secret-client"));
        assert!(!client_debug.contains("secret-refresh"));
    }

    #[tokio::test]
    async fn new_refreshes_token_before_calling_api() {
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
        let client = Client::new(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::RefreshToken {
                client_id: CLIENT_ID.to_string(),
                client_secret: CLIENT_SECRET.to_string(),
                refresh_token: REFRESH_TOKEN.to_string(),
            },
        )
        .unwrap()
        .with_root_url(base.as_str())
        .unwrap()
        .with_token_url(base.as_str())
        .unwrap();

        let token = client
            .refresh_access_token(CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN, &|_| {})
            .await
            .unwrap();
        assert_eq!(token, "fresh-token");
        client
            .upload(&token, b"FAKE".to_vec(), &|_| {})
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn start_upload_posts_to_correct_url_with_auth_and_octet_stream() {
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

        let client = client_for(&server);
        client
            .upload(TEST_TOKEN, b"FAKE_ZIP_BYTES".to_vec(), &|_| {})
            .await
            .unwrap();
    }

    // Regression guard: official V2 curl example sends neither X-Goog-Upload-Protocol
    // nor X-Goog-Upload-File-Name. fregante does, but we follow the official example.
    #[tokio::test]
    async fn start_upload_does_not_send_x_goog_upload_headers() {
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

        let client = client_for(&server);
        client
            .upload(TEST_TOKEN, b"FAKE_ZIP_BYTES".to_vec(), &|_| {})
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
    async fn start_upload_returns_upload_state_from_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "IN_PROGRESS",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let resp = client
            .upload(TEST_TOKEN, b"FAKE".to_vec(), &|_| {})
            .await
            .unwrap();
        assert!(!resp);
    }

    #[tokio::test]
    async fn start_upload_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .upload(TEST_TOKEN, b"FAKE".to_vec(), &|_| {})
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
    async fn start_upload_errors_when_initial_state_is_failed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "FAILED",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .upload(TEST_TOKEN, b"FAKE".to_vec(), &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::ChromeUpload { upload_state } => {
                assert_eq!(upload_state, "FAILED");
            }
            other => panic!("expected WepubError::ChromeUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_polls_until_succeeded() {
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

        let client = client_for(&server);
        client.await_upload(TEST_TOKEN, &|_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn await_upload_errors_when_polling_returns_failed() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "FAILED",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.await_upload(TEST_TOKEN, &|_| {}).await.unwrap_err();
        match err {
            WepubError::ChromeUpload { upload_state } => {
                assert_eq!(upload_state, "FAILED");
            }
            other => panic!("expected WepubError::ChromeUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_errors_when_polling_response_omits_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "name": "publishers/publisher-1/items/item-1",
                "itemId": "item-1",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.await_upload(TEST_TOKEN, &|_| {}).await.unwrap_err();
        match err {
            WepubError::ChromeUpload { upload_state } => {
                assert_eq!(upload_state, "UPLOAD_STATE_UNSPECIFIED");
            }
            other => panic!("expected WepubError::ChromeUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_errors_when_polling_returns_not_found() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "NOT_FOUND",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.await_upload(TEST_TOKEN, &|_| {}).await.unwrap_err();
        match err {
            WepubError::ChromeUpload { upload_state } => {
                assert_eq!(upload_state, "NOT_FOUND");
            }
            other => panic!("expected WepubError::ChromeUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "lastAsyncUploadState": "IN_PROGRESS",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client.await_upload(TEST_TOKEN, &|_| {}).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.to_lowercase().contains("timeout") || msg.to_lowercase().contains("timed out"),
            "expected timeout error, got: {msg}",
        );
    }

    #[test]
    fn upload_processed_classifies_states() {
        assert!(upload_processed(Some(UploadState::Succeeded)).unwrap());
        assert!(!upload_processed(Some(UploadState::InProgress)).unwrap());

        for (state, expected) in [
            (Some(UploadState::Failed), "FAILED"),
            (Some(UploadState::NotFound), "NOT_FOUND"),
            (None, "UPLOAD_STATE_UNSPECIFIED"),
        ] {
            match upload_processed(state).unwrap_err() {
                WepubError::ChromeUpload { upload_state } => {
                    assert_eq!(upload_state, expected);
                }
                other => panic!("expected WepubError::ChromeUpload, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn submit_default_sends_minimal_body() {
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

        let client = client_for(&server);
        client
            .submit(TEST_TOKEN, &PublishOptions::new(), &|_| {})
            .await
            .unwrap();

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
    async fn submit_staged_sends_publish_type() {
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

        let mut opts = PublishOptions::new();
        opts.publish_type = Some(PublishType::StagedPublish);

        let client = client_for(&server);
        client.submit(TEST_TOKEN, &opts, &|_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn submit_with_skip_review_and_deploy_percentage() {
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

        let mut opts = PublishOptions::new();
        opts.skip_review = Some(true);
        opts.deploy_percentage = Some(50);

        let client = client_for(&server);
        client.submit(TEST_TOKEN, &opts, &|_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn submit_errors_on_rejected_state() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "REJECTED",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .submit(TEST_TOKEN, &PublishOptions::new(), &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::ChromePublish { item_state } => {
                assert_eq!(item_state, "REJECTED");
            }
            other => panic!("expected WepubError::ChromePublish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_errors_on_cancelled_state() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/v2/publishers/publisher-1/items/item-1:publish"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "itemId": "item-1",
                "state": "CANCELLED",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .submit(TEST_TOKEN, &PublishOptions::new(), &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::ChromePublish { item_state } => {
                assert_eq!(item_state, "CANCELLED");
            }
            other => panic!("expected WepubError::ChromePublish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_accepts_non_terminal_item_states() {
        for wire in [
            "PENDING_REVIEW",
            "STAGED",
            "PUBLISHED",
            "PUBLISHED_TO_TESTERS",
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "itemId": "item-1",
                    "state": wire,
                })))
                .mount(&server)
                .await;

            let client = client_for(&server);
            client
                .submit(TEST_TOKEN, &PublishOptions::new(), &|_| {})
                .await
                .unwrap_or_else(|e| panic!("wire value {wire} should succeed, got {e:?}"));
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

        let client = client_for(&server);
        let progress = std::sync::Mutex::new(Vec::new());
        client
            .publish(b"FAKE_ZIP_BYTES".to_vec(), PublishOptions::new(), |p| {
                progress.lock().unwrap().push(p);
            })
            .await
            .unwrap();
        assert_eq!(
            progress.into_inner().unwrap(),
            [Progress::Upload, Progress::AwaitUpload, Progress::Submit,],
        );
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

        let client = client_for(&server);
        let progress = std::sync::Mutex::new(Vec::new());
        client
            .publish(b"FAKE_ZIP_BYTES".to_vec(), PublishOptions::new(), |p| {
                progress.lock().unwrap().push(p);
            })
            .await
            .unwrap();
        assert_eq!(
            progress.into_inner().unwrap(),
            [Progress::Upload, Progress::Submit],
        );
    }

    #[test]
    fn with_root_url_rejects_garbage() {
        let client = Client::new(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("token".to_string()),
        )
        .unwrap();
        let Err(err) = client.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    #[test]
    fn with_token_url_rejects_garbage() {
        let client = Client::new(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("token".to_string()),
        )
        .unwrap();
        let Err(err) = client.with_token_url("not a url") else {
            panic!("expected with_token_url to reject");
        };
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    fn client_for(server: &MockServer) -> Client {
        let base = server.uri();
        Client::new(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("test-access-token".to_string()),
        )
        .unwrap()
        .with_root_url(&base)
        .unwrap()
        .with_token_url(&base)
        .unwrap()
        .with_poll_config(PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        })
    }
}
