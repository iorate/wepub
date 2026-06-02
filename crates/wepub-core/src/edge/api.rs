use std::fmt;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, parse_root_url, pretty_json, send_request},
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

impl fmt::Debug for Credentials {
    // Redact contents.
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
pub enum Progress {
    /// Uploading the package archive.
    Uploading,
    /// Polling the upload status.
    PollingUpload,
    /// Publishing the draft.
    Publishing,
    /// Polling the publish status.
    PollingPublish,
    /// Publishing succeeded.
    Succeeded,
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

    /// Upload `zip` and publish the draft.
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
    pub async fn publish(
        &self,
        zip: Vec<u8>,
        options: PublishOptions,
        on_progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<()> {
        let on_progress = &on_progress as &(dyn Fn(Progress) + Send + Sync);

        let upload_operation_id = self.upload(zip, on_progress).await?;
        self.wait_until_uploaded(&upload_operation_id, on_progress)
            .await?;

        let publish_operation_id = self
            .do_publish(options.notes.as_deref(), on_progress)
            .await?;
        self.wait_until_published(&publish_operation_id, on_progress)
            .await?;

        on_progress(Progress::Succeeded);
        Ok(())
    }

    async fn upload(
        &self,
        zip: Vec<u8>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<String> {
        on_progress(Progress::Uploading);

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

        extract_operation_id(resp).await
    }

    async fn wait_until_uploaded(
        &self,
        operation_id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        let started = Instant::now();

        loop {
            on_progress(Progress::PollingUpload);

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
                Some(OperationStatus::Succeeded) => return Ok(()),
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeUploadFailed {
                        product_id: self.product_id.clone(),
                        operation: pretty_json(&operation),
                    });
                }
                Some(OperationStatus::InProgress) => {}
            }

            let elapsed = started.elapsed();
            if elapsed >= self.poll_config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_config.interval).await;
        }
    }

    async fn do_publish(
        &self,
        notes: Option<&str>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<String> {
        on_progress(Progress::Publishing);

        let mut req = self
            .http
            .post(self.endpoint(&format!("v1/products/{}/submissions", self.product_id))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.credentials.client_id);
        if let Some(notes) = notes {
            // Docs disagree (reference page says plain text, using page says
            // JSON); wdzeng/edge-addon reports plain text "worked":
            // https://github.com/wdzeng/edge-addon/pull/11#issuecomment-2503315960
            req = req.body(notes.to_string());
        }
        let req = req.build()?;

        let resp = send_request(&self.http, req).await?;

        extract_operation_id(resp).await
    }

    async fn wait_until_published(
        &self,
        operation_id: &str,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        let started = Instant::now();

        loop {
            on_progress(Progress::PollingPublish);

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
                Some(OperationStatus::Succeeded) => return Ok(()),
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgePublishFailed {
                        product_id: self.product_id.clone(),
                        operation: pretty_json(&operation),
                    });
                }
                Some(OperationStatus::InProgress) => {}
            }

            let elapsed = started.elapsed();
            if elapsed >= self.poll_config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_config.interval).await;
        }
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("ApiKey {}", self.credentials.api_key)
    }
}

// The wire format is PascalCase (`InProgress` / `Succeeded` / `Failed`),
// which matches Rust's idiomatic variant casing, so no `#[serde(rename_all)]`
// is needed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum OperationStatus {
    InProgress,
    Succeeded,
    Failed,
}

// The "Unexpected" shape documented for the publish endpoint lacks `status`;
// serde fills it with `None` so callers can distinguish it from a regular
// response.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct OperationResponse {
    id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    created_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_updated_time: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status: Option<OperationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    errors: Option<Vec<serde_json::Value>>,
}

async fn extract_operation_id(resp: reqwest::Response) -> Result<String> {
    let status = resp.status();
    // Clone the Location header before `text()` consumes the response.
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
        let op_id = client.upload(b"FAKE_ZIP".to_vec(), &|_| {}).await.unwrap();
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
        let err = client.upload(b"FAKE".to_vec(), &|_| {}).await.unwrap_err();
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
        let err = client.upload(b"FAKE".to_vec(), &|_| {}).await.unwrap_err();
        assert!(
            matches!(err, WepubError::UnexpectedResponse { .. }),
            "got {err:?}"
        );
    }

    #[tokio::test]
    async fn wait_until_uploaded_polls_until_succeeded() {
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
        client.wait_until_uploaded("op-1", &|_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_uploaded_errors_on_failed_status() {
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
        let err = client
            .wait_until_uploaded("op-2", &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::EdgeUploadFailed {
                product_id,
                operation,
            } => {
                assert_eq!(product_id, PRODUCT_ID);
                assert!(
                    operation.contains("Package validation failed"),
                    "operation: {operation}"
                );
                assert!(
                    operation.contains("InvalidPackage"),
                    "operation: {operation}"
                );
                assert!(
                    operation.contains("manifest broken"),
                    "operation: {operation}"
                );
            }
            other => panic!("expected WepubError::EdgeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_handles_unexpected_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "up-u",
                "message": "An error occurred while processing the request. Please contact support",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .wait_until_uploaded("up-u", &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::EdgeUploadFailed {
                product_id,
                operation,
            } => {
                assert_eq!(product_id, PRODUCT_ID);
                assert!(
                    operation.contains("contact support"),
                    "operation: {operation}"
                );
            }
            other => panic!("expected WepubError::EdgeUploadFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_uploaded_times_out() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "op-3",
                "status": "InProgress",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .wait_until_uploaded("op-3", &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::PollTimeout { .. } => {}
            other => panic!("expected WepubError::PollTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn do_publish_posts_notes_as_plain_text_body() {
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
            .do_publish(Some("for reviewers"), &|_| {})
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
    async fn do_publish_sends_empty_body_when_notes_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/products/{PRODUCT_ID}/submissions")))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "publish-op-2"))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server);
        let op_id = client.do_publish(None, &|_| {}).await.unwrap();
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
    async fn wait_until_published_polls_until_succeeded() {
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
        client.wait_until_published("pub-1", &|_| {}).await.unwrap();
    }

    #[tokio::test]
    async fn wait_until_published_errors_on_each_known_error_code() {
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
            let err = client
                .wait_until_published("pub-x", &|_| {})
                .await
                .unwrap_err();
            match err {
                WepubError::EdgePublishFailed {
                    product_id,
                    operation,
                } => {
                    assert_eq!(product_id, PRODUCT_ID);
                    assert!(
                        operation.contains(message),
                        "operation missing message for {code}: {operation}"
                    );
                    assert!(
                        operation.contains(code),
                        "operation missing errorCode {code}: {operation}"
                    );
                }
                other => panic!("expected WepubError::EdgePublishFailed for {code}, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn wait_until_published_handles_unexpected_shape() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "pub-u",
                "message": "An error occurred while processing the request. Please contact support",
            })))
            .mount(&server)
            .await;

        let client = client_for(&server);
        let err = client
            .wait_until_published("pub-u", &|_| {})
            .await
            .unwrap_err();
        match err {
            WepubError::EdgePublishFailed {
                product_id,
                operation,
            } => {
                assert_eq!(product_id, PRODUCT_ID);
                assert!(
                    operation.contains("contact support"),
                    "operation: {operation}"
                );
            }
            other => panic!("expected WepubError::EdgePublishFailed, got {other:?}"),
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
        client
            .publish(b"FAKE_ZIP".to_vec(), options, |_| {})
            .await
            .unwrap();
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
