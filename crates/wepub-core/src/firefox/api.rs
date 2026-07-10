use std::collections::HashMap;
use std::fmt;
use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use tracing::{Level, info, info_span, instrument, warn};
use url::Url;

use crate::{
    Result, WepubError,
    common::{
        decode_response, ensure_trailing_slash, instrument_step, join_endpoint, send_request,
    },
    http::build_client,
};

use super::auth::generate_jwt;

const DEFAULT_ROOT_URL: &str = "https://addons.mozilla.org/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// API credentials passed to [`publish`].
///
/// Obtain them from the
/// [API Credentials Management Page](https://addons.mozilla.org/developers/addon/api/key/).
#[derive(Clone)]
pub struct Credentials {
    /// API key (JWT issuer).
    pub api_key: String,
    /// API secret (JWT secret).
    pub api_secret: String,
}

// Hand-written so secrets never reach `Debug` output.
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials").finish_non_exhaustive()
    }
}

/// Version channel. Determines visibility on the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Listed publicly on Firefox Add-ons.
    Listed,
    /// Not listed on Firefox Add-ons; intended for self-distribution.
    Unlisted,
}

impl Channel {
    fn as_str(self) -> &'static str {
        match self {
            Channel::Listed => "listed",
            Channel::Unlisted => "unlisted",
        }
    }
}

/// Compatibility declaration.
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Compatibility {
    /// Shorthand form: list the compatible apps; for the version range, the
    /// manifest min/max or defaults are used.
    Shorthand(Vec<Application>),
    /// Full form: per-app explicit version range.
    Full(HashMap<Application, VersionRange>),
}

/// Application identifier used in compatibility declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Application {
    /// Desktop Firefox.
    Firefox,
    /// Firefox for Android.
    Android,
}

/// Explicit `min` / `max` application version pair used by
/// [`Compatibility::Full`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize)]
pub struct VersionRange {
    /// Minimum compatible application version. When `None`, the manifest
    /// min or default is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    /// Maximum compatible application version. When `None`, the manifest
    /// max or default is used.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

/// Publish `zip` to the Firefox add-on `addon_id` under `channel`,
/// authenticating with the supplied `credentials`.
///
/// Returns a [`Publish`] builder: configure it with the setter methods,
/// then `.await` it to upload the package and submit the new version.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::firefox::{Channel, Credentials, publish};
///
/// let zip = std::fs::read("./addon.zip")?;
/// publish(
///     "myaddon@example.com".into(),
///     Credentials {
///         api_key: "user:12345:6789".into(),
///         api_secret: "jwt-secret".into(),
///     },
///     zip,
///     Channel::Listed,
/// )
/// .await?;
/// # Ok(())
/// # }
/// ```
pub fn publish(
    addon_id: String,
    credentials: Credentials,
    zip: Vec<u8>,
    channel: Channel,
) -> Publish {
    Publish {
        addon_id,
        credentials,
        zip,
        channel,
        compatibility: None,
        approval_notes: None,
        release_notes: None,
        source: None,
        root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
        poll_interval: DEFAULT_POLL_INTERVAL,
        poll_timeout: DEFAULT_POLL_TIMEOUT,
    }
}

/// A pending publish to Firefox Add-ons, created by [`publish`].
///
/// Runs when `.await`ed; nothing is sent until then.
#[must_use = "a publish does nothing unless awaited"]
pub struct Publish {
    addon_id: String,
    credentials: Credentials,
    zip: Vec<u8>,
    channel: Channel,
    compatibility: Option<Compatibility>,
    approval_notes: Option<String>,
    release_notes: Option<HashMap<String, String>>,
    source: Option<Vec<u8>>,
    root_url: Url,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl Publish {
    /// Application compatibility declarations.
    pub fn compatibility(mut self, compatibility: Compatibility) -> Self {
        self.compatibility = Some(compatibility);
        self
    }

    /// Information for Mozilla reviewers.
    pub fn approval_notes(mut self, approval_notes: String) -> Self {
        self.approval_notes = Some(approval_notes);
        self
    }

    /// Release notes keyed by locale code.
    pub fn release_notes(mut self, release_notes: HashMap<String, String>) -> Self {
        self.release_notes = Some(release_notes);
        self
    }

    /// Source archive to attach to the version.
    pub fn source(mut self, source: Vec<u8>) -> Self {
        self.source = Some(source);
        self
    }

    /// Override the Firefox Add-ons API root URL.
    ///
    /// Defaults to `https://addons.mozilla.org/`. A trailing slash is
    /// appended to the path when missing.
    pub fn root_url(mut self, root_url: Url) -> Self {
        self.root_url = ensure_trailing_slash(root_url);
        self
    }

    /// Override the delay between successive polls for the upload result.
    pub fn poll_interval(mut self, poll_interval: Duration) -> Self {
        self.poll_interval = poll_interval;
        self
    }

    /// Override the maximum total time to wait for the upload result.
    pub fn poll_timeout(mut self, poll_timeout: Duration) -> Self {
        self.poll_timeout = poll_timeout;
        self
    }

    async fn run(mut self) -> Result<()> {
        let http = build_client()?;
        let zip = std::mem::take(&mut self.zip);
        let source = self.source.take();
        self.publish(&http, zip, source).await
    }

    #[instrument(
        skip_all,
        fields(
            store = "firefox",
            addon_id = self.addon_id.as_str(),
            channel = self.channel.as_str(),
        )
    )]
    async fn publish(
        &self,
        http: &reqwest::Client,
        zip: Vec<u8>,
        source: Option<Vec<u8>>,
    ) -> Result<()> {
        let (upload_uuid, processed) =
            instrument_step(info_span!("upload"), Level::ERROR, self.upload(http, zip)).await?;
        if !processed {
            instrument_step(
                info_span!("await_upload", upload_uuid = upload_uuid.as_str()),
                Level::ERROR,
                self.await_upload(http, &upload_uuid),
            )
            .await?;
        }

        let version_id = instrument_step(
            info_span!("create_version", upload_uuid = upload_uuid.as_str()),
            Level::ERROR,
            self.create_version(http, upload_uuid),
        )
        .await?;
        if let Some(source) = source
            && instrument_step(
                info_span!("update_version_source", version_id = version_id),
                // The version is already created, so a source failure doesn't
                // fail the publish; record it as a warning, not an error.
                Level::WARN,
                self.update_version_source(http, version_id, source),
            )
            .await
            .is_err()
        {
            warn!(version_id, "failed to update the source archive");
        }
        Ok(())
    }

    async fn upload(&self, http: &reqwest::Client, zip: Vec<u8>) -> Result<(String, bool)> {
        info!("uploading the package archive");

        let len = zip.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(zip), len)
            .file_name("addon.zip")
            .mime_str("application/zip")
            .expect("\"application/zip\" is a valid MIME type");
        let form = Form::new()
            .part("upload", part)
            .text("channel", self.channel.as_str());
        let req = http
            .post(self.endpoint("api/v5/addons/upload/")?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .multipart(form)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;

        let upload = decode_response(resp).await?;
        let processed = upload_processed(&upload)?;

        info!(
            upload_uuid = upload.uuid.as_str(),
            upload_processed = processed,
            "the package archive uploaded",
        );
        Ok((upload.uuid, processed))
    }

    async fn await_upload(&self, http: &reqwest::Client, upload_uuid: &str) -> Result<()> {
        info!("waiting for the upload to be processed");

        let started = Instant::now();

        loop {
            let elapsed = started.elapsed();
            if elapsed >= self.poll_timeout {
                return Err(WepubError::PollTimeout { elapsed });
            }
            tokio::time::sleep(self.poll_interval).await;

            let req = http
                .get(self.endpoint(&format!("api/v5/addons/upload/{upload_uuid}/"))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .build()
                .map_err(WepubError::http)?;

            let resp = send_request(http, req).await?;

            let upload: UploadResponse = decode_response(resp).await?;
            let processed = upload_processed(&upload)?;
            if processed {
                break;
            }
        }

        info!("the upload processed");
        Ok(())
    }

    async fn create_version(&self, http: &reqwest::Client, upload_uuid: String) -> Result<u64> {
        info!("creating the new version");

        let body = VersionCreateBody {
            upload: upload_uuid,
            compatibility: self.compatibility.clone(),
            approval_notes: self.approval_notes.clone(),
            release_notes: self.release_notes.clone(),
        };
        let req = http
            .post(self.endpoint(&format!("api/v5/addons/addon/{}/versions/", self.addon_id))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .json(&body)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;

        let version: VersionResponse = decode_response(resp).await?;

        info!(version_id = version.id, "the new version created");
        Ok(version.id)
    }

    async fn update_version_source(
        &self,
        http: &reqwest::Client,
        version_id: u64,
        source: Vec<u8>,
    ) -> Result<()> {
        info!("updating the source archive");

        let len = source.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(source), len)
            .file_name("source.zip")
            .mime_str("application/zip")
            .expect("\"application/zip\" is a valid MIME type");
        let form = Form::new().part("source", part);
        let req = http
            .patch(self.endpoint(&format!(
                "api/v5/addons/addon/{}/versions/{version_id}/",
                self.addon_id
            ))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .multipart(form)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;

        let _: VersionResponse = decode_response(resp).await?;

        info!("the source archive updated");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        let token = generate_jwt(&self.credentials.api_key, &self.credentials.api_secret);
        format!("JWT {token}")
    }
}

impl IntoFuture for Publish {
    type Output = Result<()>;
    type IntoFuture = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.run())
    }
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    uuid: String,
    processed: bool,
    valid: bool,
    #[serde(default)]
    validation: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct VersionCreateBody {
    upload: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    compatibility: Option<Compatibility>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_notes: Option<HashMap<String, String>>,
}

#[derive(Debug, Deserialize)]
struct VersionResponse {
    id: u64,
}

fn upload_processed(upload: &UploadResponse) -> Result<bool> {
    if upload.processed {
        if upload.valid {
            Ok(true)
        } else if let Some(validation) = upload.validation.as_ref() {
            Err(WepubError::FirefoxUpload {
                validation: validation.clone(),
            })
        } else {
            Err(WepubError::UnexpectedResponse {
                reason: "missing validation field".to_string(),
            })
        }
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn debug_redacts_secrets() {
        let credentials = Credentials {
            api_key: "issuer".to_string(),
            api_secret: "secret-jwt".to_string(),
        };
        assert!(!format!("{credentials:?}").contains("secret-jwt"));
    }

    #[tokio::test]
    async fn start_upload_posts_multipart_and_parses_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v5/addons/upload/"))
            .and(header_exists("authorization"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("abc-123", false, false)),
            )
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        let resp = p.upload(&http, b"fake-zip".to_vec()).await.unwrap();

        assert_eq!(resp.0, "abc-123");
        assert!(!resp.1);
    }

    #[tokio::test]
    async fn await_upload_returns_when_processed() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-1/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-1", false, false)),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-1/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-1", true, true)),
            )
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        p.await_upload(&http, "uuid-1").await.unwrap();
    }

    #[tokio::test]
    async fn await_upload_errors_on_invalid_validation() {
        let server = MockServer::start().await;
        let body = json!({
            "uuid": "uuid-2",
            "channel": "listed",
            "processed": true,
            "submitted": false,
            "url": "https://example.com/upload/uuid-2/",
            "valid": false,
            "validation": { "messages": [{ "type": "error", "message": "manifest broken" }] },
            "version": null,
        });
        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-2/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        let err = p.await_upload(&http, "uuid-2").await.unwrap_err();

        match err {
            WepubError::FirefoxUpload { validation } => {
                assert!(validation.to_string().contains("manifest broken"));
            }
            other => panic!("expected WepubError::FirefoxUpload, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_upload_times_out_when_processing_never_completes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-3/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-3", false, false)),
            )
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        let err = p.await_upload(&http, "uuid-3").await.unwrap_err();

        match err {
            WepubError::PollTimeout { .. } => {}
            other => panic!("expected WepubError::PollTimeout, got {other:?}"),
        }
    }

    #[test]
    fn upload_processed_classifies_states() {
        let not_processed = UploadResponse {
            uuid: "u".into(),
            processed: false,
            valid: false,
            validation: None,
        };
        assert!(!upload_processed(&not_processed).unwrap());

        let processed_valid = UploadResponse {
            uuid: "u".into(),
            processed: true,
            valid: true,
            validation: None,
        };
        assert!(upload_processed(&processed_valid).unwrap());

        let processed_invalid = UploadResponse {
            uuid: "u".into(),
            processed: true,
            valid: false,
            validation: Some(json!({ "messages": ["manifest broken"] })),
        };
        match upload_processed(&processed_invalid).unwrap_err() {
            WepubError::FirefoxUpload { validation } => {
                assert!(validation.to_string().contains("manifest broken"));
            }
            other => panic!("expected WepubError::FirefoxUpload, got {other:?}"),
        }

        let invalid_without_validation = UploadResponse {
            uuid: "u".into(),
            processed: true,
            valid: false,
            validation: None,
        };
        match upload_processed(&invalid_without_validation).unwrap_err() {
            WepubError::UnexpectedResponse { .. } => {}
            other => panic!("expected WepubError::UnexpectedResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_version_posts_json_and_parses_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v5/addons/addon/test-addon/versions/"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 4242 })))
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        let resp = p.create_version(&http, "uuid-x".to_string()).await.unwrap();

        assert_eq!(resp, 4242);
    }

    #[tokio::test]
    async fn patch_version_source_sends_multipart_patch() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v5/addons/addon/test-addon/versions/4242/"))
            .and(header_exists("authorization"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 4242 })))
            .expect(1)
            .mount(&server)
            .await;

        let p = publish_for(&server, Vec::new(), Channel::Listed);
        let http = http_client();
        p.update_version_source(&http, 4242, b"source-zip".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_runs_full_flow_when_source_is_provided() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/upload/"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("uuid-pub", false, false)),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-pub/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-pub", true, true)),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 7777 })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/api/v5/addons/addon/test-addon/versions/7777/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7777 })))
            .expect(1)
            .mount(&server)
            .await;

        publish_for(&server, b"zip".to_vec(), Channel::Listed)
            .source(b"source-zip".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_skips_source_patch_when_no_source_provided() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/upload/"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("uuid-ns", true, true)),
            )
            .expect(1)
            .mount(&server)
            .await;

        // The upload POST already reported the archive processed and valid, so
        // the polling GET must not run.
        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-ns/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-ns", true, true)),
            )
            .expect(0)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9999 })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/api/v5/addons/addon/test-addon/versions/9999/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 9999 })))
            .expect(0)
            .mount(&server)
            .await;

        publish_for(&server, b"zip".to_vec(), Channel::Listed)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_succeeds_when_source_attach_fails() {
        // The version is created and goes to review even if the source archive
        // fails to attach, so a failed PATCH is logged but not propagated.
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/upload/"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("uuid-src", true, true)),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/api/v5/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 5555 })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/api/v5/addons/addon/test-addon/versions/5555/"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .expect(1)
            .mount(&server)
            .await;

        publish_for(&server, b"zip".to_vec(), Channel::Listed)
            .source(b"source-zip".to_vec())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn publish_propagates_upload_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v5/addons/upload/"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1)
            .mount(&server)
            .await;

        let err = publish_for(&server, b"zip".to_vec(), Channel::Listed)
            .await
            .unwrap_err();

        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 401);
                assert_eq!(body, "unauthorized");
            }
            other => panic!("expected WepubError::HttpStatus, got {other:?}"),
        }
    }

    #[test]
    fn channel_serialises_as_amo_expects() {
        assert_eq!(Channel::Listed.as_str(), "listed");
        assert_eq!(Channel::Unlisted.as_str(), "unlisted");
    }

    #[test]
    fn version_create_body_minimal_only_has_upload() {
        let json = body_to_json("uuid-123".to_string(), None, None, None);
        assert_eq!(json, json!({ "upload": "uuid-123" }));
    }

    #[test]
    fn version_create_body_with_apps_shorthand() {
        let compat = Compatibility::Shorthand(vec![Application::Firefox, Application::Android]);
        let json = body_to_json("uuid-123".to_string(), Some(compat), None, None);
        assert_eq!(
            json,
            json!({
                "upload": "uuid-123",
                "compatibility": ["firefox", "android"],
            })
        );
    }

    #[test]
    fn version_create_body_with_detailed_compatibility_omits_empty_min_max() {
        let mut map = HashMap::new();
        map.insert(
            Application::Firefox,
            VersionRange {
                min: Some("58.0".into()),
                max: Some("120.0".into()),
            },
        );
        map.insert(
            Application::Android,
            VersionRange {
                min: Some("58.0".into()),
                max: None,
            },
        );
        let compat = Compatibility::Full(map);
        let json = body_to_json("uuid-123".to_string(), Some(compat), None, None);

        assert_eq!(json["upload"], "uuid-123");
        assert_eq!(
            json["compatibility"]["firefox"],
            json!({ "min": "58.0", "max": "120.0" })
        );
        assert_eq!(json["compatibility"]["android"], json!({ "min": "58.0" }));
    }

    #[test]
    fn version_create_body_includes_release_notes_and_approval_notes() {
        let mut notes = HashMap::new();
        notes.insert("en-US".into(), "Hello".into());
        notes.insert("ja".into(), "こんにちは".into());

        let json = body_to_json(
            "uuid-123".to_string(),
            None,
            Some("for reviewers".to_string()),
            Some(notes),
        );

        assert_eq!(json["upload"], "uuid-123");
        assert_eq!(json["release_notes"]["en-US"], "Hello");
        assert_eq!(json["release_notes"]["ja"], "こんにちは");
        assert_eq!(json["approval_notes"], "for reviewers");
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let p = publish(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
            Vec::new(),
            Channel::Listed,
        );
        let url = p.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(
            url.as_str(),
            "https://addons.mozilla.org/api/v5/addons/upload/"
        );
    }

    #[test]
    fn root_url_overrides_default() {
        let p = publish(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
            Vec::new(),
            Channel::Listed,
        )
        .root_url(Url::parse("http://127.0.0.1:8000/").unwrap());
        let url = p.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8000/api/v5/addons/upload/");
    }

    fn http_client() -> reqwest::Client {
        build_client().unwrap()
    }

    fn publish_for(server: &MockServer, zip: Vec<u8>, channel: Channel) -> Publish {
        publish(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
            zip,
            channel,
        )
        .root_url(Url::parse(&server.uri()).unwrap())
        .poll_interval(Duration::from_millis(10))
        .poll_timeout(Duration::from_millis(200))
    }

    fn upload_json(uuid: &str, processed: bool, valid: bool) -> serde_json::Value {
        json!({
            "uuid": uuid,
            "channel": "listed",
            "processed": processed,
            "submitted": false,
            "url": format!("https://example.com/upload/{uuid}/"),
            "valid": valid,
            "validation": null,
            "version": "1.0.0",
        })
    }

    fn body_to_json(
        upload: String,
        compatibility: Option<Compatibility>,
        approval_notes: Option<String>,
        release_notes: Option<HashMap<String, String>>,
    ) -> serde_json::Value {
        serde_json::to_value(VersionCreateBody {
            upload,
            compatibility,
            approval_notes,
            release_notes,
        })
        .unwrap()
    }
}
