use std::collections::HashMap;
use std::time::{Duration, Instant};

use bon::builder;
use serde::{Deserialize, Serialize};
use tracing::{Level, info, info_span, instrument, warn};
use url::Url;

use crate::{
    Result, WepubError,
    common::{decode_response, instrument_step, join_endpoint, send_request},
    http::build_client,
    multipart::Form,
};

use super::auth::generate_jwt;

const DEFAULT_ROOT_URL: &str = "https://addons.mozilla.org/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

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
#[derive(Debug, Clone)]
pub enum Compatibility {
    /// Shorthand form: list the compatible apps; for the version range, the
    /// manifest min/max or defaults are used.
    Shorthand(Vec<Application>),
    /// Full form: per-app explicit version range.
    Full(HashMap<Application, VersionRange>),
}

/// Application identifier used in compatibility declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Application {
    /// Desktop Firefox.
    Firefox,
    /// Firefox for Android.
    Android,
}

impl Application {
    fn as_str(self) -> &'static str {
        match self {
            Application::Firefox => "firefox",
            Application::Android => "android",
        }
    }
}

/// Explicit `min` / `max` application version pair used by
/// [`Compatibility::Full`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VersionRange {
    /// Minimum compatible application version. When `None`, the manifest
    /// min or default is used.
    pub min: Option<String>,
    /// Maximum compatible application version. When `None`, the manifest
    /// max or default is used.
    pub max: Option<String>,
}

/// Publish a package to Firefox Add-ons.
///
/// Returns a builder: set the required parameters and any options with the
/// setter methods, then finish with `call()` to upload the package and
/// submit the new version.
///
/// # Examples
///
/// ```no_run
/// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
/// use wepub_core::firefox::{Channel, publish};
///
/// let package = std::fs::read("./addon.zip")?;
/// publish()
///     .addon_id("myaddon@example.com")
///     .api_key("user:12345:6789")
///     .api_secret("jwt-secret")
///     .package(package)
///     .channel(Channel::Listed)
///     .call()
///     .await?;
/// # Ok(())
/// # }
/// ```
#[builder(on(String, into))]
pub async fn publish(
    /// Add-on ID (slug or GUID).
    addon_id: String,
    /// API key (JWT issuer).
    ///
    /// Obtain it from the
    /// [API Credentials Management Page](https://addons.mozilla.org/developers/addon/api/key/).
    api_key: String,
    /// API secret (JWT secret).
    ///
    /// Obtain it from the
    /// [API Credentials Management Page](https://addons.mozilla.org/developers/addon/api/key/).
    api_secret: String,
    /// Package archive (zip) to upload.
    package: Vec<u8>,
    /// Version channel. Determines visibility on the site.
    channel: Channel,
    /// Application compatibility declarations.
    compatibility: Option<Compatibility>,
    /// Information for Mozilla reviewers.
    approval_notes: Option<String>,
    /// Release notes keyed by locale code.
    release_notes: Option<HashMap<String, String>>,
    /// Source archive to attach to the version.
    source: Option<Vec<u8>>,
    /// Override the Firefox Add-ons API root URL.
    ///
    /// Defaults to `https://addons.mozilla.org/`. A trailing slash is
    /// appended to the path when missing.
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
        addon_id,
        api_key,
        api_secret,
        channel,
        compatibility,
        approval_notes,
        release_notes,
        root_url,
        poll_interval,
        poll_timeout,
    };
    let http = build_client()?;
    publish.publish(&http, package, source).await
}

struct Publish {
    addon_id: String,
    api_key: String,
    api_secret: String,
    channel: Channel,
    compatibility: Option<Compatibility>,
    approval_notes: Option<String>,
    release_notes: Option<HashMap<String, String>>,
    root_url: Url,
    poll_interval: Duration,
    poll_timeout: Duration,
}

impl Publish {
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
        package: Vec<u8>,
        source: Option<Vec<u8>>,
    ) -> Result<()> {
        let (upload_uuid, processed) = instrument_step(
            info_span!("upload"),
            Level::ERROR,
            self.upload(http, package),
        )
        .await?;
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

    async fn upload(&self, http: &reqwest::Client, package: Vec<u8>) -> Result<(String, bool)> {
        info!("uploading the package archive");

        let (content_type, body) = Form::new()
            .file("upload", "addon.zip", "application/zip", &package)
            .text("channel", self.channel.as_str())
            .finish();
        let req = http
            .post(self.endpoint("api/v5/addons/upload/"))
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
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
                .get(self.endpoint(&format!("api/v5/addons/upload/{upload_uuid}/")))
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
            compatibility: self.compatibility.as_ref().map(Into::into),
            approval_notes: self.approval_notes.clone(),
            release_notes: self.release_notes.clone(),
        };
        let req = http
            .post(self.endpoint(&format!("api/v5/addons/addon/{}/versions/", self.addon_id)))
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

        let (content_type, body) = Form::new()
            .file("source", "source.zip", "application/zip", &source)
            .finish();
        let req = http
            .patch(self.endpoint(&format!(
                "api/v5/addons/addon/{}/versions/{version_id}/",
                self.addon_id
            )))
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .header(reqwest::header::CONTENT_TYPE, content_type)
            .body(body)
            .build()
            .map_err(WepubError::http)?;

        let resp = send_request(http, req).await?;

        let _: VersionResponse = decode_response(resp).await?;

        info!("the source archive updated");
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Url {
        join_endpoint(&self.root_url, path)
    }

    fn auth_header(&self) -> String {
        let token = generate_jwt(&self.api_key, &self.api_secret);
        format!("JWT {token}")
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
    compatibility: Option<CompatibilityBody>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_notes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_notes: Option<HashMap<String, String>>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum CompatibilityBody {
    Shorthand(Vec<&'static str>),
    Full(HashMap<&'static str, VersionRangeBody>),
}

impl From<&Compatibility> for CompatibilityBody {
    fn from(compatibility: &Compatibility) -> Self {
        match compatibility {
            Compatibility::Shorthand(apps) => {
                CompatibilityBody::Shorthand(apps.iter().map(|app| app.as_str()).collect())
            }
            Compatibility::Full(ranges) => CompatibilityBody::Full(
                ranges
                    .iter()
                    .map(|(app, range)| {
                        (
                            app.as_str(),
                            VersionRangeBody {
                                min: range.min.clone(),
                                max: range.max.clone(),
                            },
                        )
                    })
                    .collect(),
            ),
        }
    }
}

#[derive(Serialize)]
struct VersionRangeBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max: Option<String>,
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

        let p = publish_for(&server);
        let http = http_client();
        let resp = p.upload(&http, b"fake-zip".to_vec()).await.unwrap();

        assert_eq!(resp.0, "abc-123");
        assert!(!resp.1);

        let received = server.received_requests().await.unwrap();
        let req = &received[0];
        let content_type = req.headers.get("content-type").unwrap().to_str().unwrap();
        let boundary = content_type
            .strip_prefix("multipart/form-data; boundary=")
            .unwrap_or_else(|| panic!("unexpected content type: {content_type}"));
        let body = std::str::from_utf8(&req.body).unwrap();
        let expected = format!(
            "--{boundary}\r\n\
             Content-Disposition: form-data; name=\"upload\"; filename=\"addon.zip\"\r\n\
             Content-Type: application/zip\r\n\
             \r\n\
             fake-zip\r\n\
             --{boundary}\r\n\
             Content-Disposition: form-data; name=\"channel\"\r\n\
             \r\n\
             listed\r\n\
             --{boundary}--\r\n"
        );
        assert_eq!(body, expected);
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

        let p = publish_for(&server);
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

        let p = publish_for(&server);
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

        let p = publish_for(&server);
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

        let p = publish_for(&server);
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

        let p = publish_for(&server);
        let http = http_client();
        p.update_version_source(&http, 4242, b"source-zip".to_vec())
            .await
            .unwrap();

        let received = server.received_requests().await.unwrap();
        let req = &received[0];
        let content_type = req.headers.get("content-type").unwrap().to_str().unwrap();
        assert!(
            content_type.starts_with("multipart/form-data; boundary="),
            "unexpected content type: {content_type}",
        );
        let body = std::str::from_utf8(&req.body).unwrap();
        assert!(
            body.contains(
                "Content-Disposition: form-data; name=\"source\"; filename=\"source.zip\""
            ),
            "body: {body}",
        );
        assert!(body.contains("source-zip"), "body: {body}");
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

        run_publish(&server, Some(b"source-zip".to_vec()))
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

        run_publish(&server, None).await.unwrap();
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

        run_publish(&server, Some(b"source-zip".to_vec()))
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

        let err = run_publish(&server, None).await.unwrap_err();

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
        let json = body_to_json("uuid-123".to_string(), Some(&compat), None, None);
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
        let json = body_to_json("uuid-123".to_string(), Some(&compat), None, None);

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
        let p = publish_for_default_root();
        let url = p.endpoint("api/v5/addons/upload/");
        assert_eq!(
            url.as_str(),
            "https://addons.mozilla.org/api/v5/addons/upload/"
        );
    }

    #[tokio::test]
    async fn root_url_without_trailing_slash_is_normalized() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/prefix/api/v5/addons/upload/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(upload_json(
                "uuid-slash",
                true,
                true,
            )))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/prefix/api/v5/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 1 })))
            .expect(1)
            .mount(&server)
            .await;

        publish()
            .addon_id("test-addon")
            .api_key("issuer")
            .api_secret("secret")
            .package(b"zip".to_vec())
            .channel(Channel::Listed)
            .root_url(Url::parse(&format!("{}/prefix", server.uri())).unwrap())
            .poll_interval(Duration::from_millis(10))
            .poll_timeout(Duration::from_millis(200))
            .call()
            .await
            .unwrap();
    }

    fn http_client() -> reqwest::Client {
        build_client().unwrap()
    }

    fn publish_for(server: &MockServer) -> Publish {
        let mut p = publish_for_default_root();
        p.root_url = Url::parse(&server.uri()).unwrap();
        p
    }

    fn publish_for_default_root() -> Publish {
        Publish {
            addon_id: "test-addon".to_string(),
            api_key: "issuer".to_string(),
            api_secret: "secret".to_string(),
            channel: Channel::Listed,
            compatibility: None,
            approval_notes: None,
            release_notes: None,
            root_url: Url::parse(DEFAULT_ROOT_URL).unwrap(),
            poll_interval: Duration::from_millis(10),
            poll_timeout: Duration::from_millis(200),
        }
    }

    async fn run_publish(server: &MockServer, source: Option<Vec<u8>>) -> Result<()> {
        publish()
            .addon_id("test-addon")
            .api_key("issuer")
            .api_secret("secret")
            .package(b"zip".to_vec())
            .channel(Channel::Listed)
            .maybe_source(source)
            .root_url(Url::parse(&server.uri()).unwrap())
            .poll_interval(Duration::from_millis(10))
            .poll_timeout(Duration::from_millis(200))
            .call()
            .await
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
        compatibility: Option<&Compatibility>,
        approval_notes: Option<String>,
        release_notes: Option<HashMap<String, String>>,
    ) -> serde_json::Value {
        serde_json::to_value(VersionCreateBody {
            upload,
            compatibility: compatibility.map(Into::into),
            approval_notes,
            release_notes,
        })
        .unwrap()
    }
}
