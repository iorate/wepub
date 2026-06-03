use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, parse_root_url, send_request, to_pretty_string},
    http::build_client,
};

const DEFAULT_ROOT_URL: &str = "https://api.addons.microsoftedge.microsoft.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// API credentials passed to [`Client::new`].
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

/// Options that shape how [`Client::publish`] submits the new version.
#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    /// Notes for certification.
    pub notes: Option<String>,
}

impl PublishOptions {
    /// Build a `PublishOptions` with all fields unset.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

/// Progress events reported by [`Client::publish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Progress {
    /// Uploading the package archive.
    StartUpload,
    /// Waiting for the upload to be processed.
    AwaitUpload,
    /// Submitting the draft.
    StartSubmit,
    /// Waiting for the submission to be processed.
    AwaitSubmit,
}

/// Client for the Edge Add-ons API (v1.1).
#[derive(Debug, Clone)]
pub struct Client {
    product_id: String,
    credentials: Credentials,
    root_url: Url,
    poll_config: PollConfig,
    http: reqwest::Client,
}

impl Client {
    /// Build a client bound to `product_id`, authenticating with the
    /// supplied `credentials`.
    pub fn new(product_id: String, credentials: Credentials) -> Result<Self> {
        Ok(Self {
            product_id,
            credentials,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            poll_config: PollConfig {
                interval: DEFAULT_POLL_INTERVAL,
                timeout: DEFAULT_POLL_TIMEOUT,
            },
            http: build_client()?,
        })
    }

    /// Override the Edge Add-ons API root URL.
    ///
    /// Defaults to `https://api.addons.microsoftedge.microsoft.com/`.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Override the poll config used.
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
    /// # async fn run() -> wepub_core::Result<()> {
    /// use wepub_core::edge::{Client, Credentials, PublishOptions};
    ///
    /// let client = Client::new(
    ///     "d34f98f5-f9b7-42b1-bebb-98707202b21d".into(),
    ///     Credentials {
    ///         client_id: "client-id".into(),
    ///         api_key: "api-key".into(),
    ///     },
    /// )?;
    /// let zip = std::fs::read("./extension.zip")?;
    /// client.publish(zip, PublishOptions::new(), |_progress| {}).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[tracing::instrument(skip_all, fields(store = "Edge Add-ons", product_id = %self.product_id))]
    pub async fn publish(
        &self,
        zip: Vec<u8>,
        options: PublishOptions,
        on_progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<()> {
        let on_progress = &on_progress as &(dyn Fn(Progress) + Send + Sync);

        let operation_id = self.start_upload(zip, on_progress).await?;
        self.await_upload(&operation_id, on_progress).await?;

        let operation_id = self.start_submit(options.notes, on_progress).await?;
        self.await_submit(&operation_id, on_progress).await?;

        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn start_upload(
        &self,
        zip: Vec<u8>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<String> {
        on_progress(Progress::StartUpload);
        tracing::info!("uploading the package archive");

        let req = self
            .http
            .post(self.endpoint(&format!(
                "v1/products/{}/submissions/draft/package",
                self.product_id
            ))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.credentials.client_id)
            .header(reqwest::header::CONTENT_TYPE, "application/zip")
            .body(zip)
            .build()?;

        let resp = send_request(&self.http, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        tracing::info!(operation_id = %operation_id, "the package archive uploaded");
        Ok(operation_id)
    }

    #[tracing::instrument(skip_all, fields(operation_id))]
    async fn await_upload(
        &self,
        operation_id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        on_progress(Progress::AwaitUpload);
        tracing::info!("waiting for the upload to be processed");

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
                    "v1/products/{}/submissions/draft/package/operations/{operation_id}",
                    self.product_id
                ))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.credentials.client_id)
                .build()?;

            let resp = send_request(&self.http, req).await?;

            let operation: OperationResponse = decode_response(resp).await?;
            match operation.status {
                Some(OperationStatus::Succeeded) => break,
                Some(OperationStatus::InProgress) => {}
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeUploadFailed {
                        error: to_pretty_string(&OperationError {
                            error_code: operation.error_code,
                            message: operation.message,
                            errors: operation.errors,
                        }),
                    });
                }
            }
        }

        tracing::info!("the upload processed");
        Ok(())
    }

    #[tracing::instrument(skip_all)]
    async fn start_submit(
        &self,
        notes: Option<String>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<String> {
        on_progress(Progress::StartSubmit);
        tracing::info!("submitting the draft");

        let mut req = self
            .http
            .post(self.endpoint(&format!("v1/products/{}/submissions", self.product_id))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.credentials.client_id);
        if let Some(notes) = notes {
            // Docs disagree (reference page says plain text, using page says
            // JSON); wdzeng/edge-addon reports plain text "worked":
            // https://github.com/wdzeng/edge-addon/pull/11#issuecomment-2503315960
            req = req.body(notes);
        }
        let req = req.build()?;

        let resp = send_request(&self.http, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        tracing::info!(operation_id = %operation_id, "the draft submitted");
        Ok(operation_id)
    }

    #[tracing::instrument(skip_all, fields(operation_id))]
    async fn await_submit(
        &self,
        operation_id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        on_progress(Progress::AwaitSubmit);
        tracing::info!("waiting for the submission to be processed");

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
                    "v1/products/{}/submissions/operations/{operation_id}",
                    self.product_id
                ))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.credentials.client_id)
                .build()?;

            let resp = send_request(&self.http, req).await?;

            let operation: OperationResponse = decode_response(resp).await?;
            match operation.status {
                Some(OperationStatus::Succeeded) => break,
                Some(OperationStatus::InProgress) => {}
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeSubmissionFailed {
                        error: to_pretty_string(&OperationError {
                            error_code: operation.error_code,
                            message: operation.message,
                            errors: operation.errors,
                        }),
                    });
                }
            }
        }

        tracing::info!("the submission processed");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("ApiKey {}", self.credentials.api_key)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<OperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<serde_json::Value>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OperationError {
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<serde_json::Value>>,
}

async fn extract_operation_id(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    let location = resp.headers().get(reqwest::header::LOCATION).cloned();
    let body = resp.text().await?;
    tracing::debug!(
        status = status.as_u16(),
        body = %body,
        "received response",
    );
    if !status.is_success() {
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    let location = location.ok_or_else(|| WepubError::UnexpectedResponse {
        detail: "missing Location header".to_string(),
    })?;
    let operation_id = location
        .to_str()
        .map_err(|_| WepubError::UnexpectedResponse {
            detail: "non-ASCII Location header".to_string(),
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

        let client = Client::new(PRODUCT_ID.to_string(), credentials).unwrap();
        assert!(!format!("{client:?}").contains("secret-key"));
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

        let client = client_for(&server);
        let op_id = client
            .start_upload(b"FAKE_ZIP".to_vec(), &|_| {})
            .await
            .unwrap();
        assert_eq!(op_id, "operation-abc-123");
    }

    #[tokio::test]
    async fn upload_propagates_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .start_upload(b"FAKE".to_vec(), &|_| {})
            .await
            .unwrap_err();
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

        let client = client_for(&server);
        let err = client
            .start_upload(b"FAKE".to_vec(), &|_| {})
            .await
            .unwrap_err();
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

        let client = client_for(&server);
        client.await_upload("op-1", &|_| {}).await.unwrap();
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

        let client = client_for(&server);
        let err = client.await_upload("op-2", &|_| {}).await.unwrap_err();
        match err {
            WepubError::EdgeUploadFailed { error } => {
                assert!(
                    error.contains("Package validation failed"),
                    "error: {error}"
                );
                assert!(error.contains("InvalidPackage"), "error: {error}");
                assert!(error.contains("manifest broken"), "error: {error}");
            }
            other => panic!("expected WepubError::EdgeUploadFailed, got {other:?}"),
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

        let client = client_for(&server);
        let err = client.await_upload("up-u", &|_| {}).await.unwrap_err();
        match err {
            WepubError::EdgeUploadFailed { error } => {
                assert!(error.contains("contact support"), "error: {error}");
            }
            other => panic!("expected WepubError::EdgeUploadFailed, got {other:?}"),
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

        let client = client_for(&server);
        let err = client.await_upload("op-3", &|_| {}).await.unwrap_err();
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

        let client = client_for(&server);
        let op_id = client
            .start_submit(Some("for reviewers".to_string()), &|_| {})
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

        let client = client_for(&server);
        let op_id = client.start_submit(None, &|_| {}).await.unwrap();
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

        let client = client_for(&server);
        client.await_submit("pub-1", &|_| {}).await.unwrap();
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

            let client = client_for(&server);
            let err = client.await_submit("pub-x", &|_| {}).await.unwrap_err();
            match err {
                WepubError::EdgeSubmissionFailed { error } => {
                    assert!(
                        error.contains(message),
                        "error missing message for {code}: {error}"
                    );
                    assert!(
                        error.contains(code),
                        "error missing errorCode {code}: {error}"
                    );
                }
                other => {
                    panic!("expected WepubError::EdgeSubmissionFailed for {code}, got {other:?}")
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

        let client = client_for(&server);
        let err = client.await_submit("pub-u", &|_| {}).await.unwrap_err();
        match err {
            WepubError::EdgeSubmissionFailed { error } => {
                assert!(error.contains("contact support"), "error: {error}");
            }
            other => panic!("expected WepubError::EdgeSubmissionFailed, got {other:?}"),
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

        let client = client_for(&server);
        let options = PublishOptions {
            notes: Some("ship it".into()),
        };
        let progress = std::sync::Mutex::new(Vec::new());
        client
            .publish(b"FAKE_ZIP".to_vec(), options, |p| {
                progress.lock().unwrap().push(p);
            })
            .await
            .unwrap();
        assert_eq!(
            progress.into_inner().unwrap(),
            [
                Progress::StartUpload,
                Progress::AwaitUpload,
                Progress::StartSubmit,
                Progress::AwaitSubmit,
            ],
        );
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let client = Client::new(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
        )
        .unwrap();
        let url = client.endpoint("v1/products/p/submissions").unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.addons.microsoftedge.microsoft.com/v1/products/p/submissions"
        );
    }

    #[test]
    fn with_root_url_overrides_default() {
        let client = Client::new(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
        )
        .unwrap()
        .with_root_url("http://127.0.0.1:8000/")
        .unwrap();
        let url = client.endpoint("v1/products/p/submissions").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:8000/v1/products/p/submissions"
        );
    }

    #[test]
    fn with_root_url_rejects_garbage() {
        let client = Client::new(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
        )
        .unwrap();
        let Err(err) = client.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    fn client_for(server: &MockServer) -> Client {
        Client::new(
            PRODUCT_ID.into(),
            Credentials {
                client_id: CLIENT_ID.into(),
                api_key: API_KEY.into(),
            },
        )
        .unwrap()
        .with_root_url(&server.uri())
        .unwrap()
        .with_poll_config(PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        })
    }
}
