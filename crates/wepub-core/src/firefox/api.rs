use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    PollConfig, Result, WepubError,
    common::{decode_response, join_endpoint, parse_root_url, send_request},
    http::build_client,
};

use super::auth::generate_jwt;

const DEFAULT_ROOT_URL: &str = "https://addons.mozilla.org/";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// API credentials passed to [`Client::new`].
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

/// Options that shape how [`Client::publish`] creates the new version.
#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    /// Application compatibility declarations.
    pub compatibility: Option<Compatibility>,
    /// Information for Mozilla reviewers.
    pub approval_notes: Option<String>,
    /// Release notes keyed by locale code.
    pub release_notes: Option<HashMap<String, String>>,
    /// Source archive to attach to the version.
    pub source: Option<Vec<u8>>,
}

impl PublishOptions {
    /// Build a `PublishOptions` with all fields unset.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
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

/// Progress events reported by [`Client::publish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Progress {
    /// Uploading the package archive.
    Upload,
    /// Waiting for the upload to be processed.
    AwaitUpload,
    /// Creating the new version.
    CreateVersion,
    /// Updating the source archive.
    UpdateVersionSource,
}

/// Client for the Firefox Add-ons API (v5).
#[derive(Debug, Clone)]
pub struct Client {
    addon_id: String,
    credentials: Credentials,
    root_url: Url,
    poll_config: PollConfig,
    http: reqwest::Client,
}

impl Client {
    /// Build a client bound to `addon_id`, authenticating with the supplied
    /// `credentials`.
    pub fn new(addon_id: String, credentials: Credentials) -> Result<Self> {
        Ok(Self {
            addon_id,
            credentials,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            poll_config: PollConfig {
                interval: DEFAULT_POLL_INTERVAL,
                timeout: DEFAULT_POLL_TIMEOUT,
            },
            http: build_client()?,
        })
    }

    /// Override the Firefox Add-ons API root URL.
    ///
    /// Defaults to `https://addons.mozilla.org/`.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Override the poll config.
    #[must_use]
    pub fn with_poll_config(mut self, poll_config: PollConfig) -> Self {
        self.poll_config = poll_config;
        self
    }

    /// Upload `zip` and submit the new version under `channel`.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> Result<(), Box<dyn std::error::Error>> {
    /// use wepub_core::firefox::{Channel, Client, Credentials, PublishOptions};
    ///
    /// let client = Client::new(
    ///     "myaddon@example.com".into(),
    ///     Credentials {
    ///         api_key: "user:12345:6789".into(),
    ///         api_secret: "jwt-secret".into(),
    ///     },
    /// )?;
    /// let zip = std::fs::read("./addon.zip")?;
    /// client.publish(zip, Channel::Listed, PublishOptions::new(), |_progress| {}).await?;
    /// # Ok(())
    /// # }
    /// ```
    #[tracing::instrument(
        skip_all,
        fields(store = "firefox", addon_id = self.addon_id.as_str(), channel = channel.as_str())
    )]
    pub async fn publish(
        &self,
        zip: Vec<u8>,
        channel: Channel,
        options: PublishOptions,
        on_progress: impl Fn(Progress) + Send + Sync,
    ) -> Result<()> {
        let on_progress = &on_progress as &(dyn Fn(Progress) + Send + Sync);

        let (upload_uuid, processed) = self.upload(zip, channel, on_progress).await?;
        if !processed {
            self.await_upload(&upload_uuid, on_progress).await?;
        }

        let version_id = self
            .create_version(
                upload_uuid,
                options.compatibility,
                options.approval_notes,
                options.release_notes,
                on_progress,
            )
            .await?;
        if let Some(source) = options.source
            && self
                .update_version_source(version_id, source, on_progress)
                .await
                .is_err()
        {
            // The version is already created, so don't fail the publish.
            tracing::error!(version_id, "failed to update the source archive");
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, err)]
    async fn upload(
        &self,
        zip: Vec<u8>,
        channel: Channel,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<(String, bool)> {
        tracing::info!("uploading the package archive");
        on_progress(Progress::Upload);

        let len = zip.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(zip), len)
            .file_name("addon.zip")
            .mime_str("application/zip")
            .expect("\"application/zip\" is a valid MIME type");
        let form = Form::new()
            .part("upload", part)
            .text("channel", channel.as_str());
        let req = self
            .http
            .post(self.endpoint("api/v5/addons/upload/")?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .multipart(form)
            .build()?;

        let resp = send_request(&self.http, req).await?;

        let upload = decode_response(resp).await?;
        let processed = upload_processed(&upload)?;

        tracing::info!(
            upload_uuid = upload.uuid.as_str(),
            upload_processed = processed,
            "the package archive uploaded",
        );
        Ok((upload.uuid, processed))
    }

    #[tracing::instrument(skip_all, fields(upload_uuid = upload_uuid), err)]
    async fn await_upload(
        &self,
        upload_uuid: &str,
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
                .get(self.endpoint(&format!("api/v5/addons/upload/{upload_uuid}/"))?)
                .header(reqwest::header::AUTHORIZATION, self.auth_header())
                .build()?;

            let resp = send_request(&self.http, req).await?;

            let upload: UploadResponse = decode_response(resp).await?;
            let processed = upload_processed(&upload)?;
            if processed {
                break;
            }
        }

        tracing::info!("the upload processed");
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(upload_uuid = upload_uuid.as_str()), err)]
    async fn create_version(
        &self,
        upload_uuid: String,
        compatibility: Option<Compatibility>,
        approval_notes: Option<String>,
        release_notes: Option<HashMap<String, String>>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<u64> {
        tracing::info!("creating the new version");
        on_progress(Progress::CreateVersion);

        let body = VersionCreateBody {
            upload: upload_uuid,
            compatibility,
            approval_notes,
            release_notes,
        };
        let req = self
            .http
            .post(self.endpoint(&format!("api/v5/addons/addon/{}/versions/", self.addon_id))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .json(&body)
            .build()?;

        let resp = send_request(&self.http, req).await?;

        let version: VersionResponse = decode_response(resp).await?;

        tracing::info!(version_id = version.id, "the new version created");
        Ok(version.id)
    }

    #[tracing::instrument(skip_all, fields(version_id = version_id), err)]
    async fn update_version_source(
        &self,
        version_id: u64,
        source: Vec<u8>,
        on_progress: &(dyn Fn(Progress) + Send + Sync),
    ) -> Result<()> {
        tracing::info!("updating the source archive");
        on_progress(Progress::UpdateVersionSource);

        let len = source.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(source), len)
            .file_name("source.zip")
            .mime_str("application/zip")
            .expect("\"application/zip\" is a valid MIME type");
        let form = Form::new().part("source", part);
        let req = self
            .http
            .patch(self.endpoint(&format!(
                "api/v5/addons/addon/{}/versions/{version_id}/",
                self.addon_id
            ))?)
            .header(reqwest::header::AUTHORIZATION, self.auth_header())
            .multipart(form)
            .build()?;

        let resp = send_request(&self.http, req).await?;

        let _: VersionResponse = decode_response(resp).await?;

        tracing::info!("the source archive updated");
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

        let client = Client::new("addon-1".to_string(), credentials).unwrap();
        assert!(!format!("{client:?}").contains("secret-jwt"));
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

        let client = client_for(&server);
        let resp = client
            .upload(b"fake-zip".to_vec(), Channel::Listed, &|_| {})
            .await
            .unwrap();

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

        let client = client_for(&server);
        client.await_upload("uuid-1", &|_| {}).await.unwrap();
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

        let client = client_for(&server);
        let err = client.await_upload("uuid-2", &|_| {}).await.unwrap_err();

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

        let client = client_for(&server);
        let err = client.await_upload("uuid-3", &|_| {}).await.unwrap_err();

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

        let client = client_for(&server);
        let resp = client
            .create_version("uuid-x".to_string(), None, None, None, &|_| {})
            .await
            .unwrap();

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

        let client = client_for(&server);
        client
            .update_version_source(4242, b"source-zip".to_vec(), &|_| {})
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

        let client = client_for(&server);
        let options = PublishOptions {
            source: Some(b"source-zip".to_vec()),
            ..PublishOptions::new()
        };
        let progress = std::sync::Mutex::new(Vec::new());
        client
            .publish(b"zip".to_vec(), Channel::Listed, options, |p| {
                progress.lock().unwrap().push(p);
            })
            .await
            .unwrap();
        assert_eq!(
            progress.into_inner().unwrap(),
            [
                Progress::Upload,
                Progress::AwaitUpload,
                Progress::CreateVersion,
                Progress::UpdateVersionSource
            ],
        );
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

        let client = client_for(&server);
        let progress = std::sync::Mutex::new(Vec::new());
        client
            .publish(
                b"zip".to_vec(),
                Channel::Listed,
                PublishOptions::new(),
                |p| progress.lock().unwrap().push(p),
            )
            .await
            .unwrap();
        assert_eq!(
            progress.into_inner().unwrap(),
            [Progress::Upload, Progress::CreateVersion],
        );
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

        let client = client_for(&server);
        let options = PublishOptions {
            source: Some(b"source-zip".to_vec()),
            ..PublishOptions::new()
        };
        client
            .publish(b"zip".to_vec(), Channel::Listed, options, |_| {})
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

        let client = client_for(&server);
        let err = client
            .publish(
                b"zip".to_vec(),
                Channel::Listed,
                PublishOptions::new(),
                |_| {},
            )
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
        let client = Client::new(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
        )
        .unwrap();
        let url = client.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(
            url.as_str(),
            "https://addons.mozilla.org/api/v5/addons/upload/"
        );
    }

    #[test]
    fn with_root_url_overrides_default() {
        let client = Client::new(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
        )
        .unwrap()
        .with_root_url("http://127.0.0.1:8000/")
        .unwrap();
        let url = client.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8000/api/v5/addons/upload/");
    }

    #[test]
    fn with_root_url_rejects_garbage() {
        let client = Client::new(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
            },
        )
        .unwrap();
        let Err(err) = client.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::Url { .. }), "got {err:?}");
    }

    fn client_for(server: &MockServer) -> Client {
        Client::new(
            "test-addon".into(),
            Credentials {
                api_key: "issuer".into(),
                api_secret: "secret".into(),
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
