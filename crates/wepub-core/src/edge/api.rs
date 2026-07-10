use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::{Duration, Instant};

use serde::Deserialize;
use tracing::{Level, debug, info, info_span, instrument};
use url::Url;

use crate::{
    Result, WepubError,
    common::{
        decode_response, ensure_trailing_slash, instrument_step, join_endpoint, send_request,
    },
    http::build_client,
};

const DEFAULT_ROOT_URL: &str = "https://api.addons.microsoftedge.microsoft.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// API credentials passed to [`publish`].
///
/// Obtain them from the **Publish API** page of the
/// [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login).
#[derive(Clone)]
pub struct Credentials {
    /// Client ID.
    pub client_id: String,
    /// API key.
    pub api_key: String,
}

// Hand-written so secrets never reach `Debug` output.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

/// Publish `zip` to the Edge add-on `product_id`, authenticating with the
/// supplied `credentials`.
///
/// Returns a [`Publish`] builder: configure it with the setter methods,
/// then `.await` it to upload the package and submit the draft.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::edge::{Credentials, publish};
///
/// let zip = std::fs::read("./addon.zip")?;
/// publish(
///     "d34f98f5-f9b7-42b1-bebb-98707202b21d".into(),
///     Credentials {
///         client_id: "client-id".into(),
///         api_key: "api-key".into(),
///     },
///     zip,
/// )
/// .await?;
/// # Ok(())
/// # }
/// ```
pub fn publish(product_id: String, credentials: Credentials, zip: Vec<u8>) -> Publish {
    Publish {
        product_id,
        credentials,
        zip,
        notes: None,
        root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
        poll_interval: DEFAULT_POLL_INTERVAL,
        poll_timeout: DEFAULT_POLL_TIMEOUT,
    }
}

/// A pending publish to Edge Add-ons, created by [`publish`].
///
/// Runs when `.await`ed; nothing is sent until then.
#[must_use = "a publish does nothing unless awaited"]
pub struct Publish {
    product_id: String,
    credentials: Credentials,
    zip: Vec<u8>,
    notes: Option<String>,
    root_url: Url,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl Publish {
    /// Notes for certification.
    pub fn notes(mut self, notes: String) -> Self {
        self.notes = Some(notes);
        self
    }

    /// Override the Edge Add-ons API root URL.
    ///
    /// Defaults to `https://api.addons.microsoftedge.microsoft.com/`. A
    /// trailing slash is appended to the path when missing.
    pub fn root_url(mut self, root_url: Url) -> Self {
        self.root_url = ensure_trailing_slash(root_url);
        self
    }

    /// Override the delay between successive polls for operation results.
    pub fn poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Override the maximum total time to wait for each operation result.
    pub fn poll_timeout(mut self, poll_timeout: Duration) -> Self {
        self.poll_timeout = poll_timeout;
        self
    }

    async fn run(mut self) -> Result<()> {
        let http = build_client()?;
        let zip = std::mem::take(&mut self.zip);
        let notes = self.notes.take();
        self.publish(&http, zip, notes).await
    }

    #[instrument(skip_all, fields(store = "edge", product_id = self.product_id.as_str()))]
    async fn publish(
        &self,
        http: &reqwest::Client,
        zip: Vec<u8>,
        notes: Option<String>,
    ) -> Result<()> {
        let upload_operation_id =
            instrument_step(info_span!("upload"), Level::ERROR, self.upload(http, zip)).await?;
        instrument_step(
            info_span!(
                "await_upload",
                upload_operation_id = upload_operation_id.as_str()
            ),
            Level::ERROR,
            self.await_upload(http, &upload_operation_id),
        )
        .await?;

        let publish_operation_id =
            instrument_step(info_span!("submit"), Level::ERROR, self.submit(http, notes)).await?;
        instrument_step(
            info_span!(
                "await_submit",
                publish_operation_id = publish_operation_id.as_str()
            ),
            Level::ERROR,
            self.await_submit(http, &publish_operation_id),
        )
        .await?;

        Ok(())
    }

    async fn upload(&self, http: &reqwest::Client, zip: Vec<u8>) -> Result<String> {
        info!("uploading the package archive");

        let req = http
            .post(self.endpoint(&format!(
                "v1/products/{}/submissions/draft/package",
                self.product_id
            ))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.credentials.client_id)
            .header(reqwest::header::CONTENT_TYPE, "application/zip")
            .body(zip)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        info!(
            upload_operation_id = operation_id.as_str(),
            "the package archive uploaded"
        );
        Ok(operation_id)
    }

    async fn await_upload(&self, http: &reqwest::Client, upload_operation_id: &str) -> Result<()> {
        info!("waiting for the upload to be processed");

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_interval).await;

            let req = http
                .get(self.endpoint(&format!(
                    "v1/products/{}/submissions/draft/package/operations/{upload_operation_id}",
                    self.product_id
                ))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.credentials.client_id)
                .build()
                .map_err(WepubError::http)?;

            let resp = send_request(http, req).await?;

            let operation: OperationResponse = decode_response(resp).await?;
            match operation.status {
                Some(OperationStatus::Succeeded) => break,
                Some(OperationStatus::InProgress) => {}
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeApi {
                        message: operation.message,
                        error_code: operation.error_code,
                        errors: operation.errors,
                    });
                }
            }
        }

        info!("the upload processed");
        Ok(())
    }

    async fn submit(&self, http: &reqwest::Client, notes: Option<String>) -> Result<String> {
        info!("submitting the draft");

        let mut req = http
            .post(self.endpoint(&format!("v1/products/{}/submissions", self.product_id))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.credentials.client_id);
        if let Some(notes) = notes {
            // Docs disagree (reference page says plain text, using page says
            // JSON); wdzeng/edge-addon reports plain text "worked":
            // https://github.com/wdzeng/edge-addon/pull/11#issuecomment-2503315960
            req = req.body(notes);
        }
        let req = req.build().map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        info!(
            publish_operation_id = operation_id.as_str(),
            "the draft submitted"
        );
        Ok(operation_id)
    }

    async fn await_submit(&self, http: &reqwest::Client, publish_operation_id: &str) -> Result<()> {
        info!("waiting for the submission to be processed");

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_interval).await;

            let req = http
                .get(self.endpoint(&format!(
                    "v1/products/{}/submissions/operations/{}",
                    self.product_id, publish_operation_id
                ))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.credentials.client_id)
                .build()
                .map_err(WepubError::http)?;

            let resp = send_request(http, req).await?;

            let operation: OperationResponse = decode_response(resp).await?;
            match operation.status {
                Some(OperationStatus::Succeeded) => break,
                Some(OperationStatus::InProgress) => {}
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeApi {
                        message: operation.message,
                        error_code: operation.error_code,
                        errors: operation.errors,
                    });
                }
            }
        }

        info!("the submission processed");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("ApiKey {}", self.credentials.api_key)
    }
}

impl IntoFuture for Publish {
    type Output = Result<()>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

// The variant names are the wire format as-is (PascalCase).
#[derive(Clone, Copy, Deserialize)]
enum OperationStatus {
    InProgress,
    Succeeded,
    Failed,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationResponse {
    #[serde(default)]
    status: Option<OperationStatus>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    error_code: Option<String>,
    #[serde(default)]
    errors: Option<Vec<serde_json::Value>>,
}

async fn extract_operation_id(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    let location = resp.headers().get(reqwest::header::LOCATION).cloned();
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
    let location = location.ok_or_else(|| WepubError::UnexpectedResponse {
        reason: "missing Location header".to_string(),
    })?;
    let operation_id = location
        .to_str()
        .map_err(|_| WepubError::UnexpectedResponse {
            reason: "non-ASCII Location header".to_string(),
        })?
        .to_string();
    Ok(operation_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PRODUCT_ID: &str = "11111111-2222-3333-4444-555555555555";
    const API_KEY: &str = "test-api-key";
    const CLIENT_ID: &str = "test-client-id";

    #[test]
    fn debug_redacts_secrets() {
        let credentials = Credentials {
            client_id: "client-id".to_string(),
            api_key: "secret-key".to_string(),
        };
        assert!(!format!("{credentials:?}").contains("secret-key"));
    }

    #[tokio::test]
    async fn upload_posts_zip_with_apikey_and_clientid_headers() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/products/{PRODUCT_ID}/submissions/draft/package"
            )))
            .and(header(
                "authorization",
                format!("ApiKey {API_KEY}").as_str(),
            ))
            .and(header("x-clientid", CLIENT_ID))
            .and(header("content-type", "application/zip"))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "operation-abc-123"))
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let op_id = p.upload(&http, b"FAKE_ZIP".to_vec()).await.unwrap();
        assert_eq!(op_id, "operation-abc-123");
    }

    #[tokio::test]
    async fn upload_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.upload(&http, b"FAKE".to_vec()).await.unwrap_err();
        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 401);
                assert!(body.contains("Unauthorized"));
            }
            other => panic!("expected WepubError::HttpStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upload_fails_when_location_header_missing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(202))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.upload(&http, b"FAKE".to_vec()).await.unwrap_err();
        assert!(
            matches!(err, WepubError::UnexpectedResponse { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn await_upload_polls_until_succeeded() {
        let server = MockServer::start().await;
        let upload_op_path =
            format!("/v1/products/{PRODUCT_ID}/submissions/draft/package/operations/op-1");

        Mock::given(method("GET"))
            .and(path(upload_op_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "op-1",
                "status": "InProgress",
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(upload_op_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "op-1",
                "status": "Succeeded",
                "message": "Successfully updated package to extension.zip",
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        p.await_upload(&http, "op-1").await.unwrap();
    }

    #[tokio::test]
    async fn await_upload_errors_on_failed_status() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "op-2",
                "status": "Failed",
                "message": "Package validation failed.",
                "errorCode": "InvalidPackage",
                "errors": [{"message": "manifest broken"}],
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.await_upload(&http, "op-2").await.unwrap_err();
        match err {
            WepubError::EdgeApi {
                message,
                error_code,
                errors,
            } => {
                assert_eq!(message.unwrap(), "Package validation failed.");
                assert_eq!(error_code.unwrap(), "InvalidPackage");
                assert!(
                    errors.as_ref().unwrap()[0]
                        .to_string()
                        .contains("manifest broken"),
                    "errors: {errors:?}",
                );
            }
            other => panic!("expected WepubError::EdgeApi, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_handles_unexpected_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "up-u",
                "message": "An error occurred while processing the request. Please contact support",
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.await_upload(&http, "up-u").await.unwrap_err();
        match err {
            WepubError::EdgeApi { message, .. } => {
                assert!(
                    message.as_ref().unwrap().contains("contact support"),
                    "message: {message:?}"
                );
            }
            other => panic!("expected WepubError::EdgeApi, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "op-3",
                "status": "InProgress",
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.await_upload(&http, "op-3").await.unwrap_err();
        match err {
            WepubError::PollTimeout { .. } => {}
            other => panic!("expected WepubError::PollTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn start_submit_posts_notes_as_plain_text_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/products/{PRODUCT_ID}/submissions")))
            .and(header(
                "authorization",
                format!("ApiKey {API_KEY}").as_str(),
            ))
            .and(header("x-clientid", CLIENT_ID))
            .and(body_string("for reviewers"))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "publish-op-1"))
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let op_id = p
            .submit(&http, Some("for reviewers".to_string()))
            .await
            .unwrap();
        assert_eq!(op_id, "publish-op-1");

        let received = server.received_requests().await.unwrap();
        let req = &received[0];
        assert!(
            req.headers.get("content-type").is_none(),
            "Content-Type must not be set; got {:?}",
            req.headers.get("content-type"),
        );
    }

    #[tokio::test]
    async fn start_submit_sends_empty_body_when_notes_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/products/{PRODUCT_ID}/submissions")))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "publish-op-2"))
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let op_id = p.submit(&http, None).await.unwrap();
        assert_eq!(op_id, "publish-op-2");

        let received = server.received_requests().await.unwrap();
        let req = &received[0];
        assert!(
            req.headers.get("content-type").is_none(),
            "Content-Type must not be set when notes is None; got {:?}",
            req.headers.get("content-type"),
        );
    }

    #[tokio::test]
    async fn await_submit_polls_until_succeeded() {
        let server = MockServer::start().await;
        let publish_op_path = format!("/v1/products/{PRODUCT_ID}/submissions/operations/pub-1");

        Mock::given(method("GET"))
            .and(path(publish_op_path.clone()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pub-1",
                "status": "InProgress",
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(publish_op_path))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pub-1",
                "status": "Succeeded",
                "message": "Successfully created submission with ID 42",
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        p.await_submit(&http, "pub-1").await.unwrap();
    }

    #[tokio::test]
    async fn await_submit_errors_on_each_known_error_code() {
        let cases = [
            ("CreateNotAllowed", "Can't create new extension."),
            (
                "NoModulesUpdated",
                "Can't publish extension since there are no updates",
            ),
            (
                "InProgressSubmission",
                "Can't publish extension as your extension submission is in progress",
            ),
            (
                "UnpublishInProgress",
                "Can't publish extension as your extension is being unpublished",
            ),
            (
                "ModuleStateUnPublishable",
                "Can't publish extension as your extension has modules that are not valid",
            ),
            (
                "SubmissionValidationError",
                "Extension can't be published as there are submission validation failures",
            ),
        ];

        for (code, message) in cases {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "id": "pub-x",
                    "status": "Failed",
                    "message": message,
                    "errorCode": code,
                })))
                .mount(&server)
                .await;

            let p = publish_for(&server, Vec::new());
            let http = http_client();
            let err = p.await_submit(&http, "pub-x").await.unwrap_err();
            match err {
                WepubError::EdgeApi {
                    message: actual_message,
                    error_code: actual_code,
                    ..
                } => {
                    assert_eq!(actual_message.unwrap(), message);
                    assert_eq!(actual_code.unwrap(), code);
                }
                other => {
                    panic!("expected WepubError::EdgeApi for {code}, got {other:?}")
                }
            }
        }
    }

    #[tokio::test]
    async fn await_submit_handles_unexpected_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pub-u",
                "message": "An error occurred while processing the request. Please contact support",
            })))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new());
        let http = http_client();
        let err = p.await_submit(&http, "pub-u").await.unwrap_err();
        match err {
            WepubError::EdgeApi { message, .. } => {
                assert!(
                    message.as_ref().unwrap().contains("contact support"),
                    "message: {message:?}"
                );
            }
            other => panic!("expected WepubError::EdgeApi, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn publish_full_happy_path() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path(format!(
                "/v1/products/{PRODUCT_ID}/submissions/draft/package"
            )))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "upl-op"))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/products/{PRODUCT_ID}/submissions/draft/package/operations/upl-op"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "upl-op",
                "status": "Succeeded",
                "message": "Successfully updated package",
            })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path(format!("/v1/products/{PRODUCT_ID}/submissions")))
            .and(body_string("ship it"))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "pub-op"))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path(format!(
                "/v1/products/{PRODUCT_ID}/submissions/operations/pub-op"
            )))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pub-op",
                "status": "Succeeded",
                "message": "Successfully created submission with ID 42",
            })))
            .expect(1)
            .mount(&server)
            .await;

        publish_for(&server, b"FAKE_ZIP".to_vec())
            .notes("ship it".into())
            .await
            .unwrap();
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let p = publish(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
            Vec::new(),
        );
        let url = p.endpoint("v1/products/p/submissions").unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.addons.microsoftedge.microsoft.com/v1/products/p/submissions"
        );
    }

    #[test]
    fn root_url_overrides_default() {
        let p = publish(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
            Vec::new(),
        )
        .root_url(Url::parse("http://127.0.0.1:8000/").unwrap());
        let url = p.endpoint("v1/products/p/submissions").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8000/v1/products/p/submissions"
        );
    }

    fn http_client() -> reqwest::Client {
        build_client().unwrap()
    }

    fn publish_for(server: &MockServer, zip: Vec<u8>) -> Publish {
        publish(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
            zip,
        )
        .root_url(Url::parse(&server.uri()).unwrap())
        .poll_interval(Duration::from_millis(10))
        .poll_timeout(Duration::from_millis(200))
    }
}
