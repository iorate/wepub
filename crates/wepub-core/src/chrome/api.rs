use std::time::{Duration, Instant};

use bon::builder;
use isahc::HttpClient;
use isahc::http::{Request, header};
use serde::{Deserialize, Serialize};
use tracing::{Level, info, info_span, instrument};
use url::Url;

use crate::{
    Result, WepubError,
    http::{build_client, decode_response, join_endpoint, send_request},
    instrument::instrument_step,
};

use super::auth::{self, DEFAULT_TOKEN_URL};

const DEFAULT_ROOT_URL: &str = "https://chromewebstore.googleapis.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(2);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Whether a new version is published immediately on approval or staged for
/// later publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PublishType {
    /// Publish immediately on approval.
    DefaultPublish,
    /// Stage for later publishing.
    StagedPublish,
}

impl PublishType {
    fn as_str(self) -> &'static str {
        match self {
            PublishType::DefaultPublish => "DEFAULT_PUBLISH",
            PublishType::StagedPublish => "STAGED_PUBLISH",
        }
    }
}

/// Exchange an OAuth refresh token for an access token.
///
/// Returns a builder: set the required parameters with the setter methods,
/// then run it by awaiting the builder directly or by finishing with
/// `call()`. Obtain the client credentials and refresh token by following
/// [Use the Chrome Web Store API](https://developer.chrome.com/docs/webstore/using-api).
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::chrome::fetch_access_token;
///
/// let access_token = fetch_access_token()
///     .client_id("client-id")
///     .client_secret("client-secret")
///     .refresh_token("refresh-token")
///     .await?;
/// # Ok(())
/// # }
/// ```
#[builder(on(String, into), derive(IntoFuture(Box)))]
pub async fn fetch_access_token(
    /// OAuth client ID.
    client_id: String,
    /// OAuth client secret.
    client_secret: String,
    /// OAuth refresh token.
    refresh_token: String,
    /// Override the Google OAuth token endpoint URL.
    ///
    /// Defaults to `https://oauth2.googleapis.com/token`.
    #[builder(default = Url::parse(DEFAULT_TOKEN_URL).expect("DEFAULT_TOKEN_URL is a valid URL"))]
    token_url: Url,
) -> Result<String> {
    let client = build_client()?;
    instrument_step(
        info_span!("fetch_access_token", store = "chrome"),
        Level::ERROR,
        async {
            info!("fetching the access token");
            let token = auth::fetch_access_token(
                &client,
                token_url,
                &client_id,
                &client_secret,
                &refresh_token,
            )
            .await?;
            info!("the access token fetched");
            Ok(token)
        },
    )
    .await
}

/// Publish a package to the Chrome Web Store.
///
/// Returns a builder: set the required parameters and any options with the
/// setter methods, then run it by awaiting the builder directly or by
/// finishing with `call()` to upload the package and submit the draft.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::chrome::{PublishType, publish};
///
/// let package = std::fs::read("./extension.zip")?;
/// publish()
///     .publisher_id("abcd1234-ef56-7890-abcd-ef1234567890")
///     .item_id("abcdefghijklmnopabcdefghijklmnop")
///     .access_token("access-token")
///     .package(package)
///     .publish_type(PublishType::StagedPublish)
///     .await?;
/// # Ok(())
/// # }
/// ```
#[builder(on(String, into), derive(IntoFuture(Box)))]
pub async fn publish(
    /// Publisher ID (UUID).
    publisher_id: String,
    /// Item ID (32-character string).
    item_id: String,
    /// OAuth access token.
    ///
    /// Obtain one with [`fetch_access_token`], or from any other source,
    /// such as a
    /// [service account](https://developer.chrome.com/docs/webstore/service-accounts).
    access_token: String,
    /// Package archive (zip) to upload.
    package: Vec<u8>,
    /// Whether to publish immediately on approval or stage for later
    /// publishing.
    publish_type: Option<PublishType>,
    /// Initial percentage of users to roll the new version out to.
    ///
    /// Defaults to the value configured in the Developer Dashboard.
    deploy_percentage: Option<u8>,
    /// Attempt to skip item review.
    skip_review: Option<bool>,
    /// Override the Chrome Web Store API root URL.
    ///
    /// Defaults to `https://chromewebstore.googleapis.com/`. A trailing
    /// slash is appended to the path when missing.
    #[builder(default = Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"))]
    root_url: Url,
    /// Override the delay between successive polls for the upload result.
    #[builder(default = DEFAULT_POLL_INTERVAL)]
    poll_interval: Duration,
    /// Override the maximum total time to wait for the upload result.
    #[builder(default = DEFAULT_POLL_TIMEOUT)]
    poll_timeout: Duration,
) -> Result<()> {
    let publish = Publish {
        publisher_id,
        item_id,
        access_token,
        publish_type,
        deploy_percentage,
        skip_review,
        root_url,
        poll_interval,
        poll_timeout,
    };
    let client = build_client()?;
    publish.publish(&client, package).await
}

#[allow(clippy::struct_field_names)]
struct Publish {
    publisher_id: String,
    item_id: String,
    access_token: String,
    publish_type: Option<PublishType>,
    deploy_percentage: Option<u8>,
    skip_review: Option<bool>,
    root_url: Url,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl Publish {
    #[instrument(
        skip_all,
        fields(
            store = "chrome",
            publisher_id = self.publisher_id.as_str(),
            item_id = self.item_id.as_str(),
        )
    )]
    async fn publish(&self, client: &HttpClient, package: Vec<u8>) -> Result<()> {
        let processed = instrument_step(
            info_span!("upload"),
            Level::ERROR,
            self.upload(client, package),
        )
        .await?;
        if !processed {
            instrument_step(
                info_span!("await_upload"),
                Level::ERROR,
                self.await_upload(client),
            )
            .await?;
        }

        instrument_step(info_span!("submit"), Level::ERROR, self.submit(client)).await?;

        Ok(())
    }

    async fn upload(&self, client: &HttpClient, package: Vec<u8>) -> Result<bool> {
        info!("uploading the package archive");

        let req = Request::post(
            self.endpoint(&format!(
                "upload/v2/publishers/{}/items/{}:upload",
                self.publisher_id, self.item_id
            ))
            .as_str(),
        )
        .header(header::AUTHORIZATION, self.auth_header())
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(package)
        .map_err(WepubError::http)?;

        let resp = send_request(client, req).await?;

        let upload: UploadResponse = decode_response(resp).await?;
        let processed = upload_processed(Some(upload.upload_state))?;

        info!(
            upload_state = upload.upload_state.as_str(),
            "the package archive uploaded",
        );
        Ok(processed)
    }

    async fn await_upload(&self, client: &HttpClient) -> Result<()> {
        info!("waiting for the upload to be processed");

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            async_io::Timer::after(self.poll_interval).await;

            let req = Request::get(
                self.endpoint(&format!(
                    "v2/publishers/{}/items/{}:fetchStatus",
                    self.publisher_id, self.item_id
                ))
                .as_str(),
            )
            .header(header::AUTHORIZATION, self.auth_header())
            .body(())
            .map_err(WepubError::http)?;

            let resp = send_request(client, req).await?;

            let status: FetchStatusResponse = decode_response(resp).await?;
            let processed = upload_processed(status.last_async_upload_state)?;
            if processed {
                break;
            }
        }

        info!("the upload processed");
        Ok(())
    }

    async fn submit(&self, client: &HttpClient) -> Result<()> {
        info!("submitting the draft");

        let body = PublishRequestBody {
            publish_type: self.publish_type.map(PublishType::as_str),
            deploy_infos: self.deploy_percentage.map(|p| {
                vec![DeployInfo {
                    deploy_percentage: p,
                }]
            }),
            skip_review: self.skip_review,
        };
        let body = serde_json::to_vec(&body).expect("serializing a plain request body cannot fail");
        let req = Request::post(
            self.endpoint(&format!(
                "v2/publishers/{}/items/{}:publish",
                self.publisher_id, self.item_id
            ))
            .as_str(),
        )
        .header(header::AUTHORIZATION, self.auth_header())
        .header(header::CONTENT_TYPE, "application/json")
        .body(body)
        .map_err(WepubError::http)?;

        let resp = send_request(client, req).await?;

        let publish: PublishResponse = decode_response(resp).await?;
        if let ItemState::Rejected | ItemState::Cancelled = publish.state {
            return Err(WepubError::ChromePublish {
                item_state: publish.state.as_str().to_string(),
            });
        }

        info!(item_state = publish.state.as_str(), "the draft submitted");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Url {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("Bearer {}", self.access_token)
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
    publish_type: Option<&'static str>,
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

    #[tokio::test]
    async fn fetch_access_token_exchanges_refresh_token() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=refresh_token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "access_token": "fresh-token" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        let token = fetch_access_token()
            .client_id(CLIENT_ID)
            .client_secret(CLIENT_SECRET)
            .refresh_token(REFRESH_TOKEN)
            .token_url(Url::parse(&server.uri()).unwrap())
            .call()
            .await
            .unwrap();
        assert_eq!(token, "fresh-token");
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

        let p = publish_for(&server);
        let client = http_client();
        p.upload(&client, b"FAKE_ZIP_BYTES".to_vec()).await.unwrap();
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

        let p = publish_for(&server);
        let client = http_client();
        p.upload(&client, b"FAKE_ZIP_BYTES".to_vec()).await.unwrap();

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

        let p = publish_for(&server);
        let client = http_client();
        let resp = p.upload(&client, b"FAKE".to_vec()).await.unwrap();
        assert!(!resp);
    }

    #[tokio::test]
    async fn start_upload_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let p = publish_for(&server);
        let client = http_client();
        let err = p.upload(&client, b"FAKE".to_vec()).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.upload(&client, b"FAKE".to_vec()).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        p.await_upload(&client).await.unwrap();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        p.submit(&client).await.unwrap();

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

        let mut p = publish_for(&server);
        p.publish_type = Some(PublishType::StagedPublish);
        let client = http_client();
        p.submit(&client).await.unwrap();
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

        let mut p = publish_for(&server);
        p.skip_review = Some(true);
        p.deploy_percentage = Some(50);
        let client = http_client();
        p.submit(&client).await.unwrap();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.submit(&client).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.submit(&client).await.unwrap_err();
        match err {
            WepubError::ChromePublish { item_state } => {
                assert_eq!(item_state, "CANCELLED");
            }
            other => panic!("expected WepubError::ChromePublish, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_accepts_non_terminal_item_states() {
        for state in [
            "PENDING_REVIEW",
            "STAGED",
            "PUBLISHED",
            "PUBLISHED_TO_TESTERS",
        ] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "itemId": "item-1",
                    "state": state,
                })))
                .mount(&server)
                .await;

            let p = publish_for(&server);
            let client = http_client();
            p.submit(&client)
                .await
                .unwrap_or_else(|err| panic!("state {state} should be accepted, got {err:?}"));
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

        run_publish(&server).await.unwrap();
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

        run_publish(&server).await.unwrap();
    }

    fn http_client() -> HttpClient {
        build_client().unwrap()
    }

    fn publish_for(server: &MockServer) -> Publish {
        Publish {
            publisher_id: "publisher-1".to_string(),
            item_id: "item-1".to_string(),
            access_token: TEST_TOKEN.to_string(),
            publish_type: None,
            deploy_percentage: None,
            skip_review: None,
            root_url: Url::parse(&server.uri()).unwrap(),
            poll_interval: Duration::from_millis(10),
            poll_timeout: Duration::from_millis(200),
        }
    }

    async fn run_publish(server: &MockServer) -> Result<()> {
        publish()
            .publisher_id("publisher-1")
            .item_id("item-1")
            .access_token(TEST_TOKEN)
            .package(b"FAKE_ZIP_BYTES".to_vec())
            .root_url(Url::parse(&server.uri()).unwrap())
            .poll_interval(Duration::from_millis(10))
            .poll_timeout(Duration::from_millis(200))
            .call()
            .await
    }
}
