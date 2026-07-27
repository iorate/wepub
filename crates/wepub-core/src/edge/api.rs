use std::time::{Duration, Instant};

use bon::builder;
use isahc::http::{Request, Response, header};
use isahc::{AsyncBody, AsyncReadResponseExt, HttpClient};
use serde::Deserialize;
use tracing::{Level, debug, info, info_span, instrument};
use url::Url;

use crate::{
    Result, WepubError,
    http::{build_client, decode_response, join_endpoint, send_request},
    instrument::instrument_step,
};

const DEFAULT_ROOT_URL: &str = "https://api.addons.microsoftedge.microsoft.com/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Publish a package to Edge Add-ons.
///
/// Returns a builder: set the required parameters and any options with the
/// setter methods, then run it by awaiting the builder directly or by
/// finishing with `call()` to upload the package and submit the draft.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::edge::publish;
///
/// let package = std::fs::read("./addon.zip")?;
/// publish()
///     .product_id("d34f98f5-f9b7-42b1-bebb-98707202b21d")
///     .client_id("client-id")
///     .api_key("api-key")
///     .package(package)
///     .await?;
/// # Ok(())
/// # }
/// ```
#[builder(on(String, into), derive(IntoFuture(Box)))]
pub async fn publish(
    /// Product ID (GUID).
    product_id: String,
    /// Client ID.
    ///
    /// Obtain it from the **Publish API** page of the
    /// [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login).
    client_id: String,
    /// API key.
    ///
    /// Obtain it from the **Publish API** page of the
    /// [Partner Center developer dashboard](https://partner.microsoft.com/dashboard/microsoftedge/public/login).
    api_key: String,
    /// Package archive (zip) to upload.
    package: Vec<u8>,
    /// Notes for certification.
    notes: Option<String>,
    /// Override the Edge Add-ons API root URL.
    ///
    /// Defaults to `https://api.addons.microsoftedge.microsoft.com/`. A
    /// trailing slash is appended to the path when missing.
    #[builder(default = Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"))]
    root_url: Url,
    /// Override the delay between successive polls for operation results.
    #[builder(default = DEFAULT_POLL_INTERVAL)]
    poll_interval: Duration,
    /// Override the maximum total time to wait for each operation result.
    #[builder(default = DEFAULT_POLL_TIMEOUT)]
    poll_timeout: Duration,
) -> Result<()> {
    let publish = Publish {
        product_id,
        client_id,
        api_key,
        root_url,
        poll_interval,
        poll_timeout,
    };
    let client = build_client()?;
    publish.publish(&client, package, notes).await
}

struct Publish {
    product_id: String,
    client_id: String,
    api_key: String,
    root_url: Url,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl Publish {
    #[instrument(skip_all, fields(store = "edge", product_id = self.product_id.as_str()))]
    async fn publish(
        &self,
        client: &HttpClient,
        package: Vec<u8>,
        notes: Option<String>,
    ) -> Result<()> {
        let upload_operation_id = instrument_step(
            info_span!("upload"),
            Level::ERROR,
            self.upload(client, package),
        )
        .await?;
        instrument_step(
            info_span!(
                "await_upload",
                upload_operation_id = upload_operation_id.as_str()
            ),
            Level::ERROR,
            self.await_upload(client, &upload_operation_id),
        )
        .await?;

        let publish_operation_id = instrument_step(
            info_span!("submit"),
            Level::ERROR,
            self.submit(client, notes),
        )
        .await?;
        instrument_step(
            info_span!(
                "await_submit",
                publish_operation_id = publish_operation_id.as_str()
            ),
            Level::ERROR,
            self.await_submit(client, &publish_operation_id),
        )
        .await?;

        Ok(())
    }

    async fn upload(&self, client: &HttpClient, package: Vec<u8>) -> Result<String> {
        info!("uploading the package archive");

        let req = Request::post(
            self.endpoint(&format!(
                "v1/products/{}/submissions/draft/package",
                self.product_id
            ))
            .as_str(),
        )
        .header(header::AUTHORIZATION, self.auth_header())
        .header("X-ClientID", &self.client_id)
        .header(header::CONTENT_TYPE, "application/zip")
        .body(package)
        .map_err(WepubError::http)?;

        let resp = send_request(client, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        info!(
            upload_operation_id = operation_id.as_str(),
            "the package archive uploaded"
        );
        Ok(operation_id)
    }

    async fn await_upload(&self, client: &HttpClient, upload_operation_id: &str) -> Result<()> {
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
                    "v1/products/{}/submissions/draft/package/operations/{upload_operation_id}",
                    self.product_id
                ))
                .as_str(),
            )
            .header(header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.client_id)
            .body(())
            .map_err(WepubError::http)?;

            let resp = send_request(client, req).await?;

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

    async fn submit(&self, client: &HttpClient, notes: Option<String>) -> Result<String> {
        info!("submitting the draft");

        let builder = Request::post(
            self.endpoint(&format!("v1/products/{}/submissions", self.product_id))
                .as_str(),
        )
        .header(header::AUTHORIZATION, self.auth_header())
        .header("X-ClientID", &self.client_id);
        // Docs disagree (reference page says plain text, using page says
        // JSON); wdzeng/edge-addon reports plain text "worked":
        // https://github.com/wdzeng/edge-addon/pull/11#issuecomment-2503315960
        // Unlike wdzeng/edge-addon, which sends it as
        // application/x-www-form-urlencoded (axios's default), we send
        // text/plain.
        let req = match notes {
            Some(notes) => builder
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(AsyncBody::from(notes)),
            None => builder.body(AsyncBody::empty()),
        }
        .map_err(WepubError::http)?;

        let resp = send_request(client, req).await?;
        let operation_id = extract_operation_id(resp).await?;

        info!(
            publish_operation_id = operation_id.as_str(),
            "the draft submitted"
        );
        Ok(operation_id)
    }

    async fn await_submit(&self, client: &HttpClient, publish_operation_id: &str) -> Result<()> {
        info!("waiting for the submission to be processed");

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            async_io::Timer::after(self.poll_interval).await;

            let req = Request::get(
                self.endpoint(&format!(
                    "v1/products/{}/submissions/operations/{}",
                    self.product_id, publish_operation_id
                ))
                .as_str(),
            )
            .header(header::AUTHORIZATION, self.auth_header())
            .header("X-ClientID", &self.client_id)
            .body(())
            .map_err(WepubError::http)?;

            let resp = send_request(client, req).await?;

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

    fn endpoint(&self, path: &str) -> Url {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        format!("ApiKey {}", self.api_key)
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

async fn extract_operation_id(mut resp: Response<AsyncBody>) -> Result<String> {
    let status = resp.status();
    let location = resp.headers().get(header::LOCATION).cloned();
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

    #[tokio::test]
    async fn upload_posts_package_with_apikey_and_clientid_headers() {
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

        let p = publish_for(&server);
        let client = http_client();
        let op_id = p.upload(&client, b"FAKE_ZIP".to_vec()).await.unwrap();
        assert_eq!(op_id, "operation-abc-123");
    }

    #[tokio::test]
    async fn upload_propagates_http_error() {
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.upload(&client, b"FAKE".to_vec()).await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        p.await_upload(&client, "op-1").await.unwrap();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client, "op-2").await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client, "up-u").await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_upload(&client, "op-3").await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let op_id = p
            .submit(&client, Some("for reviewers".to_string()))
            .await
            .unwrap();
        assert_eq!(op_id, "publish-op-1");

        let received = server.received_requests().await.unwrap();
        let req = &received[0];
        assert_eq!(
            req.headers.get("content-type").unwrap(),
            "text/plain; charset=utf-8",
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

        let p = publish_for(&server);
        let client = http_client();
        let op_id = p.submit(&client, None).await.unwrap();
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

        let p = publish_for(&server);
        let client = http_client();
        p.await_submit(&client, "pub-1").await.unwrap();
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

            let p = publish_for(&server);
            let client = http_client();
            let err = p.await_submit(&client, "pub-x").await.unwrap_err();
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

        let p = publish_for(&server);
        let client = http_client();
        let err = p.await_submit(&client, "pub-u").await.unwrap_err();
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

        publish()
            .product_id(PRODUCT_ID)
            .client_id(CLIENT_ID)
            .api_key(API_KEY)
            .package(b"FAKE_ZIP".to_vec())
            .notes("ship it")
            .root_url(Url::parse(&server.uri()).unwrap())
            .poll_interval(Duration::from_millis(10))
            .poll_timeout(Duration::from_millis(200))
            .call()
            .await
            .unwrap();
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let p = publish_for_default_root();
        let url = p.endpoint("v1/products/p/submissions");
        assert_eq!(
            url.as_str(),
            "https://api.addons.microsoftedge.microsoft.com/v1/products/p/submissions"
        );
    }

    fn http_client() -> HttpClient {
        build_client().unwrap()
    }

    fn publish_for(server: &MockServer) -> Publish {
        let mut p = publish_for_default_root();
        p.root_url = Url::parse(&server.uri()).unwrap();
        p
    }

    fn publish_for_default_root() -> Publish {
        Publish {
            product_id: PRODUCT_ID.to_string(),
            client_id: CLIENT_ID.to_string(),
            api_key: API_KEY.to_string(),
            root_url: Url::parse(DEFAULT_ROOT_URL).unwrap(),
            poll_interval: Duration::from_millis(10),
            poll_timeout: Duration::from_millis(200),
        }
    }
}
