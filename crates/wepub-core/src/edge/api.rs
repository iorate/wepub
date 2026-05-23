use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, log_request, parse_root_url, pretty_json},
    http::build_client,
};

const DEFAULT_ROOT_URL: &str = "https://api.addons.microsoftedge.microsoft.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Options that shape how [`Client::publish`] submits the new version.
#[derive(Debug, Clone)]
pub struct PublishOptions {
    /// Optional notes for the Edge Add-ons certification team
    /// (reviewer-facing).
    pub notes: Option<String>,

    /// Polling cadence and overall timeout used while waiting for the
    /// asynchronous upload and publish operations to finish.
    pub poll: PollConfig,
}

impl PublishOptions {
    /// Build a `PublishOptions` with the recommended defaults
    /// (5 second poll interval, 5 minute timeout).
    #[must_use]
    pub fn new() -> Self {
        Self {
            notes: None,
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

/// Client for the Edge Add-ons Update REST API (v1.1).
///
/// The client holds the Partner Center credentials and a reusable HTTP
/// client; it is cheap to construct and intended to live for the
/// duration of a single publish run.
// Debug intentionally omitted: holds the Partner Center API key.
#[allow(clippy::struct_field_names)] // `client_id` is the Partner Center auth term
pub struct Client {
    product_id: String,
    client_id: String,
    api_key: String,
    root_url: Url,
    http: reqwest::Client,
}

impl Client {
    /// Build a client bound to `product_id`, signing requests with the
    /// API credentials issued from Partner Center (Client ID + API
    /// key).
    ///
    /// Obtain the credentials from
    /// <https://partner.microsoft.com/dashboard/microsoftedge/public/login>
    /// under **Microsoft Edge** &gt; **Publish API**.
    ///
    /// # Errors
    ///
    /// Fails if the underlying HTTP client cannot be built (e.g. rustls
    /// platform-verifier initialization fails).
    pub fn new(product_id: String, client_id: String, api_key: String) -> Result<Self> {
        Ok(Self {
            product_id,
            client_id,
            api_key,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            http: build_client()?,
        })
    }

    /// Override the Edge Add-ons API root URL.
    ///
    /// Defaults to `https://api.addons.microsoftedge.microsoft.com/`.
    /// Intended for tests that point the client at a mock server. A
    /// missing trailing slash is added automatically so that relative
    /// paths join correctly.
    ///
    /// # Errors
    ///
    /// Returns [`WepubError::InvalidUrl`] if `root_url` does not parse
    /// as a URL.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Upload `zip` and submit the resulting draft for publish.
    ///
    /// Waits for the upload to be ingested, submits the draft for
    /// publish, and waits for the publish operation to complete. The
    /// polling cadence for both waits is controlled by `options.poll`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> wepub_core::Result<()> {
    /// use wepub_core::edge::{Client, PublishOptions};
    ///
    /// let client = Client::new(
    ///     "d34f98f5-f9b7-42b1-bebb-98707202b21d".into(),
    ///     "client-id".into(),
    ///     "api-key".into(),
    /// )?;
    /// let zip = std::fs::read("./extension.zip")?;
    /// client.publish(zip, PublishOptions::new()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(&self, zip: Vec<u8>, options: PublishOptions) -> Result<()> {
        let upload_op = self.upload(zip).await?;
        self.wait_until_uploaded(&upload_op, &options.poll).await?;

        let publish_op = self.submit_for_publish(options.notes.as_deref()).await?;
        self.wait_until_published(&publish_op, &options.poll)
            .await?;

        Ok(())
    }

    async fn upload(&self, zip: Vec<u8>) -> Result<String> {
        tracing::info!(
            product_id = %self.product_id,
            "uploading"
        );

        let method = reqwest::Method::POST;
        let url = self.endpoint(&format!(
            "v1/products/{}/submissions/draft/package",
            self.product_id
        ))?;

        log_request(&method, &url);
        let resp = self
            .http
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.client_id)
            .header(reqwest::header::CONTENT_TYPE, "application/zip")
            .body(zip)
            .send()
            .await?;

        Self::extract_operation_id(resp).await
    }

    async fn wait_until_uploaded(&self, operation_id: &str, config: &PollConfig) -> Result<()> {
        let url = self.endpoint(&format!(
            "v1/products/{}/submissions/draft/package/operations/{operation_id}",
            self.product_id
        ))?;
        let started = Instant::now();

        loop {
            tracing::info!(
                product_id = %self.product_id,
                "polling upload status"
            );
            let method = reqwest::Method::GET;
            log_request(&method, &url);
            let resp = self
                .http
                .request(method, url.clone())
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.client_id)
                .send()
                .await?;
            let body: OperationResponse = decode_response(resp).await?;

            match body.status {
                Some(OperationStatus::Succeeded) => return Ok(()),
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgeUploadFailed {
                        product_id: self.product_id.clone(),
                        operation: pretty_json(&body),
                    });
                }
                Some(OperationStatus::InProgress) => {}
            }

            let elapsed = started.elapsed();
            if elapsed >= config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    async fn submit_for_publish(&self, notes: Option<&str>) -> Result<String> {
        tracing::info!(
            product_id = %self.product_id,
            "submitting for publish"
        );

        let method = reqwest::Method::POST;
        let url = self.endpoint(&format!("v1/products/{}/submissions", self.product_id))?;

        log_request(&method, &url);
        let mut request = self
            .http
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.client_id);
        if let Some(notes) = notes {
            // Docs disagree (reference page says plain text, using page says
            // JSON); wdzeng/edge-addon reports plain text "worked":
            // https://github.com/wdzeng/edge-addon/pull/11#issuecomment-2503315960
            request = request.body(notes.to_string());
        }

        let resp = request.send().await?;
        Self::extract_operation_id(resp).await
    }

    async fn wait_until_published(&self, operation_id: &str, config: &PollConfig) -> Result<()> {
        let url = self.endpoint(&format!(
            "v1/products/{}/submissions/operations/{operation_id}",
            self.product_id
        ))?;
        let started = Instant::now();

        loop {
            tracing::info!(
                product_id = %self.product_id,
                "polling publish status"
            );
            let method = reqwest::Method::GET;
            log_request(&method, &url);
            let resp = self
                .http
                .request(method, url.clone())
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .header("X-ClientID", &self.client_id)
                .send()
                .await?;
            let body: OperationResponse = decode_response(resp).await?;

            match body.status {
                Some(OperationStatus::Succeeded) => {
                    tracing::info!(
                        product_id = %self.product_id,
                        message = body.message.as_deref(),
                        "publish succeeded"
                    );
                    return Ok(());
                }
                Some(OperationStatus::Failed) | None => {
                    return Err(WepubError::EdgePublishFailed {
                        product_id: self.product_id.clone(),
                        operation: pretty_json(&body),
                    });
                }
                Some(OperationStatus::InProgress) => {}
            }

            let elapsed = started.elapsed();
            if elapsed >= config.timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("ApiKey {}", self.api_key)
    }

    async fn extract_operation_id(resp: reqwest::Response) -> Result<String> {
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await?;
            return Err(WepubError::HttpStatus {
                status: status.as_u16(),
                body,
            });
        }
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .ok_or_else(|| WepubError::UnexpectedResponse {
                detail: "202 response missing Location header".to_string(),
            })?;
        let operation_id = location
            .to_str()
            .map_err(|e| WepubError::UnexpectedResponse {
                detail: format!("Location header is not ASCII: {e}"),
            })?
            .to_string();
        Ok(operation_id)
    }
}

// The wire format is PascalCase (`InProgress` / `Succeeded` / `Failed`),
// which matches Rust's idiomatic variant casing, so no `#[serde(rename_all)]`
// is needed.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
enum OperationStatus {
    InProgress,
    Succeeded,
    Failed,
}

// The "Unexpected" shape documented for the publish endpoint lacks `status`;
// serde fills it with `None` so callers can distinguish it from a regular
// response.
#[derive(Debug, Deserialize, Serialize)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const PRODUCT_ID: &str = "11111111-2222-3333-4444-555555555555";
    const API_KEY: &str = "test-api-key";
    const CLIENT_ID: &str = "test-client-id";

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
        let op_id = client.upload(b"FAKE_ZIP".to_vec()).await.unwrap();
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
        let err = client.upload(b"FAKE".to_vec()).await.unwrap_err();
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
        let err = client.upload(b"FAKE".to_vec()).await.unwrap_err();
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
        client
            .wait_until_uploaded("op-1", &fast_poll())
            .await
            .unwrap();
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
            .wait_until_uploaded("op-2", &fast_poll())
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
            .wait_until_uploaded("up-u", &fast_poll())
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
            .wait_until_uploaded("op-3", &fast_poll())
            .await
            .unwrap_err();
        match err {
            WepubError::PollTimeout { .. } => {}
            other => panic!("expected WepubError::PollTimeout, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn submit_for_publish_posts_notes_as_plain_text_body() {
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
            .submit_for_publish(Some("for reviewers"))
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
    async fn submit_for_publish_sends_empty_body_when_notes_none() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(format!("/v1/products/{PRODUCT_ID}/submissions")))
            .and(body_string(""))
            .respond_with(ResponseTemplate::new(202).insert_header("Location", "publish-op-2"))
            .expect(1)
            .mount(&server)
            .await;

        let client = client_for(&server);
        let op_id = client.submit_for_publish(None).await.unwrap();
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
        client
            .wait_until_published("pub-1", &fast_poll())
            .await
            .unwrap();
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
                .wait_until_published("pub-x", &fast_poll())
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
            .wait_until_published("pub-u", &fast_poll())
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
            poll: fast_poll(),
        };
        client.publish(b"FAKE_ZIP".to_vec(), options).await.unwrap();
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let client = Client::new(PRODUCT_ID.into(), CLIENT_ID.into(), API_KEY.into()).unwrap();
        let url = client.endpoint("v1/products/p/submissions").unwrap();
        assert_eq!(
            url.as_str(),
            "https://api.addons.microsoftedge.microsoft.com/v1/products/p/submissions"
        );
    }

    #[test]
    fn with_root_url_overrides_default() {
        let client = Client::new(PRODUCT_ID.into(), CLIENT_ID.into(), API_KEY.into())
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
        let client = Client::new(PRODUCT_ID.into(), CLIENT_ID.into(), API_KEY.into()).unwrap();
        let Err(err) = client.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    fn client_for(server: &MockServer) -> Client {
        Client::new(PRODUCT_ID.into(), CLIENT_ID.into(), API_KEY.into())
            .unwrap()
            .with_root_url(&server.uri())
            .unwrap()
    }

    fn fast_poll() -> PollConfig {
        PollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        }
    }
}
