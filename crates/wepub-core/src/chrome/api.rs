use std::borrow::Cow;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tracing::{Level, info, info_span, instrument};
use url::Url;

use crate::{
    Result, WepubError,
    common::{
        PollConfig, decode_response, instrument_step, join_endpoint, parse_root_url, send_request,
    },
    http::build_client,
};

use super::auth::{self, DEFAULT_TOKEN_URL};

const DEFAULT_ROOT_URL: &str = "https://chromewebstore.googleapis.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// OAuth credentials passed to [`publish`].
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

/// Publish `zip` to the Chrome Web Store item `item_id` owned by
/// `publisher_id`, authenticating with the supplied `credentials`.
///
/// Returns a [`Publish`] builder: configure it with the setter methods,
/// then `.await` it to upload the package and submit the draft.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::chrome::{Credentials, PublishType, publish};
///
/// let zip = std::fs::read("./extension.zip")?;
/// publish(
///     "abcd1234-ef56-7890-abcd-ef1234567890".into(),
///     "abcdefghijklmnopabcdefghijklmnop".into(),
///     Credentials::RefreshToken {
///         client_id: "client-id".into(),
///         client_secret: "client-secret".into(),
///         refresh_token: "refresh-token".into(),
///     },
///     zip,
/// )
/// .publish_type(PublishType::StagedPublish)
/// .await?;
/// # Ok(())
/// # }
/// ```
pub fn publish(
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    zip: Vec<u8>,
) -> Publish {
    Publish {
        publisher_id,
        item_id,
        credentials,
        zip,
        options: Options::default(),
        root_url: None,
        token_url: None,
        poll_interval: None,
        poll_timeout: None,
    }
}

/// A pending publish to the Chrome Web Store, created by [`publish`].
///
/// Runs when `.await`ed; nothing is sent until then.
#[must_use = "a publish does nothing unless awaited"]
pub struct Publish {
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    zip: Vec<u8>,
    options: Options,
    root_url: Option<String>,
    token_url: Option<String>,
    poll_interval: Option<Duration>,
    poll_timeout: Option<Duration>,
}

impl Publish {
    /// Whether to publish immediately on approval or stage for later
    /// publishing.
    pub fn publish_type(mut self, publish_type: PublishType) -> Self {
        self.options.publish_type = Some(publish_type);
        self
    }

    /// Initial percentage of users to roll the new version out to.
    ///
    /// Defaults to the value configured in the Developer Dashboard.
    pub fn deploy_percentage(mut self, deploy_percentage: u8) -> Self {
        self.options.deploy_percentage = Some(deploy_percentage);
        self
    }

    /// Attempt to skip item review.
    pub fn skip_review(mut self, skip_review: bool) -> Self {
        self.options.skip_review = Some(skip_review);
        self
    }

    /// Override the Chrome Web Store API root URL.
    ///
    /// Defaults to `https://chromewebstore.googleapis.com/`.
    pub fn root_url(mut self, root_url: &str) -> Self {
        self.root_url = Some(root_url.to_string());
        self
    }

    /// Override the Google OAuth token endpoint URL.
    ///
    /// Defaults to `https://oauth2.googleapis.com/token`.
    pub fn token_url(mut self, token_url: &str) -> Self {
        self.token_url = Some(token_url.to_string());
        self
    }

    /// Override the delay between successive polls for the upload result.
    pub fn poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = Some(poll_interval);
        self
    }

    /// Override the maximum total time to wait for the upload result.
    pub fn poll_timeout(mut self, poll_timeout: Duration) -> Self {
        self.poll_timeout = Some(poll_timeout);
        self
    }

    async fn run(self) -> Result<()> {
        let mut client = Client::new(self.publisher_id, self.item_id, self.credentials)?;
        if let Some(root_url) = self.root_url.as_deref() {
            client = client.with_root_url(root_url)?;
        }
        if let Some(token_url) = self.token_url.as_deref() {
            client = client.with_token_url(token_url)?;
        }
        if let Some(interval) = self.poll_interval {
            client.poll_config.interval = interval;
        }
        if let Some(timeout) = self.poll_timeout {
            client.poll_config.timeout = timeout;
        }
        client.publish(self.zip, self.options).await
    }
}

impl IntoFuture for Publish {
    type Output = Result<()>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

#[derive(Debug, Clone, Default)]
struct Options {
    publish_type: Option<PublishType>,
    deploy_percentage: Option<u8>,
    skip_review: Option<bool>,
}

#[derive(Debug, Clone)]
struct Client {
    publisher_id: String,
    item_id: String,
    credentials: Credentials,
    root_url: Url,
    token_url: Url,
    poll_config: PollConfig,
    http: reqwest::Client,
}

impl Client {
    fn new(publisher_id: String, item_id: String, credentials: Credentials) -> Result<Self> {
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

    fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    fn with_token_url(mut self, token_url: &str) -> Result<Self> {
        self.token_url = Url::parse(token_url).map_err(|err| WepubError::Url {
            url: token_url.to_string(),
            source: err,
        })?;
        Ok(self)
    }

    #[instrument(
        skip_all,
        fields(
            store = "chrome",
            publisher_id = self.publisher_id.as_str(),
            item_id = self.item_id.as_str(),
        )
    )]
    async fn publish(&self, zip: Vec<u8>, options: Options) -> Result<()> {
        let token: Cow<'_, str> = match &self.credentials {
            Credentials::RefreshToken {
                client_id,
                client_secret,
                refresh_token,
            } => {
                let token = instrument_step(
                    info_span!("refresh_access_token"),
                    Level::ERROR,
                    self.refresh_access_token(client_id, client_secret, refresh_token),
                )
                .await?;
                Cow::Owned(token)
            }
            Credentials::AccessToken(token) => Cow::Borrowed(token),
        };

        let processed =
            instrument_step(info_span!("upload"), Level::ERROR, self.upload(&token, zip)).await?;
        if !processed {
            instrument_step(
                info_span!("await_upload"),
                Level::ERROR,
                self.await_upload(&token),
            )
            .await?;
        }

        instrument_step(
            info_span!("submit"),
            Level::ERROR,
            self.submit(&token, &options),
        )
        .await?;

        Ok(())
    }

    async fn refresh_access_token(
        &self,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Result<String> {
        info!("refreshing the access token");

        let token = auth::refresh_access_token(
            &self.http,
            self.token_url.clone(),
            client_id,
            client_secret,
            refresh_token,
        )
        .await?;

        info!("the access token refreshed");
        Ok(token)
    }

    async fn upload(&self, token: &str, zip: Vec<u8>) -> Result<bool> {
        info!("uploading the package archive");

        let req = self
            .http
            .post(self.endpoint(&format!(
                "upload/v2/publishers/{}/items/{}:upload",
                self.publisher_id, self.item_id
            ))?)
            .bearer_auth(token)
            .header(reqwest::header::CONTENT_TYPE, "application/octet-stream")
            .body(zip)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(&self.http, req).await?;

        let upload: UploadResponse = decode_response(resp).await?;
        let processed = upload_processed(Some(upload.upload_state))?;

        info!(
            upload_state = upload.upload_state.as_str(),
            "the package archive uploaded",
        );
        Ok(processed)
    }

    async fn await_upload(&self, token: &str) -> Result<()> {
        info!("waiting for the upload to be processed");

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
                .build()
                .map_err(WepubError::http)?;

            let resp = send_request(&self.http, req).await?;

            let status: FetchStatusResponse = decode_response(resp).await?;
            let processed = upload_processed(status.last_async_upload_state)?;
            if processed {
                break;
            }
        }

        info!("the upload processed");
        Ok(())
    }

    async fn submit(&self, token: &str, options: &Options) -> Result<()> {
        info!("submitting the draft");

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
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(&self.http, req).await?;

        let publish: PublishResponse = decode_response(resp).await?;
        if let ItemState::Rejected | ItemState::Cancelled = publish.state {
            return Err(WepubError::ChromePublish {
                item_state: publish.state.as_str().to_string(),
            });
        }

        info!(item_state = publish.state.as_str(), "the draft submitted");
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
            .refresh_access_token(CLIENT_ID, CLIENT_SECRET, REFRESH_TOKEN)
            .await
            .unwrap();
        assert_eq!(token, "fresh-token");
        client.upload(&token, b"FAKE".to_vec()).await.unwrap();
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
            .upload(TEST_TOKEN, b"FAKE_ZIP_BYTES".to_vec())
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
    async fn start_upload_returns_upload_state_from_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "uploadState": "IN_PROGRESS",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let resp = client.upload(TEST_TOKEN, b"FAKE".to_vec()).await.unwrap();
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
            .upload(TEST_TOKEN, b"FAKE".to_vec())
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
        client.await_upload(TEST_TOKEN).await.unwrap();
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
        let err = client.await_upload(TEST_TOKEN).await.unwrap_err();
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
        let err = client.await_upload(TEST_TOKEN).await.unwrap_err();
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
        let err = client.await_upload(TEST_TOKEN).await.unwrap_err();
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
        let err = client.await_upload(TEST_TOKEN).await.unwrap_err();
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
            .submit(TEST_TOKEN, &Options::default())
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

        let opts = Options {
            publish_type: Some(PublishType::StagedPublish),
            ..Options::default()
        };

        let client = client_for(&server);
        client.submit(TEST_TOKEN, &opts).await.unwrap();
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

        let opts = Options {
            skip_review: Some(true),
            deploy_percentage: Some(50),
            ..Options::default()
        };

        let client = client_for(&server);
        client.submit(TEST_TOKEN, &opts).await.unwrap();
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
            .submit(TEST_TOKEN, &Options::default())
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
            .submit(TEST_TOKEN, &Options::default())
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
                .submit(TEST_TOKEN, &Options::default())
                .await
                .unwrap_or_else(|err| panic!("wire value {wire} should succeed, got {err:?}"));
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

        publish_for(&server, b"FAKE_ZIP_BYTES".to_vec())
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

        publish_for(&server, b"FAKE_ZIP_BYTES".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_rejects_garbage_root_url() {
        let err = publish(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("token".to_string()),
            b"FAKE".to_vec(),
        )
        .root_url("not a url")
        .await
        .unwrap_err();
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    #[tokio::test]
    async fn publish_rejects_garbage_token_url() {
        let err = publish(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("token".to_string()),
            b"FAKE".to_vec(),
        )
        .token_url("not a url")
        .await
        .unwrap_err();
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    fn client_for(server: &MockServer) -> Client {
        let base = server.uri();
        let mut client = Client::new(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("test-access-token".to_string()),
        )
        .unwrap()
        .with_root_url(&base)
        .unwrap()
        .with_token_url(&base)
        .unwrap();
        client.poll_config = PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        };
        client
    }

    fn publish_for(server: &MockServer, zip: Vec<u8>) -> Publish {
        let base = server.uri();
        publish(
            "publisher-1".to_string(),
            "item-1".to_string(),
            Credentials::AccessToken("test-access-token".to_string()),
            zip,
        )
        .root_url(&base)
        .token_url(&base)
        .poll_interval(Duration::from_millis(10))
        .poll_timeout(Duration::from_millis(200))
    }
}
