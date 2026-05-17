use std::collections::HashMap;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    Phase, Result, Store, WepubError,
    common::{decode_response, join_endpoint, log_request, parse_root_url, pretty_json},
    http::build_client,
};

use super::auth::generate_jwt;

const DEFAULT_ROOT_URL: &str = "https://addons.mozilla.org/";
const UPLOAD_FILE_NAME: &str = "addon.zip";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Options that shape how [`FirefoxStore::publish`] creates the new version.
#[derive(Debug, Clone, Default)]
pub struct FirefoxPublishOptions {
    /// Distribution channel for the new version.
    pub channel: Channel,
    /// Application compatibility declarations. `None` falls back to whatever
    /// the manifest's `strict_min_version` / `strict_max_version` declare.
    pub compatibility: Option<Compatibility>,
    /// Release notes keyed by AMO locale code (e.g. `"en-US"`).
    pub release_notes: HashMap<String, String>,
    /// Optional message to AMO reviewers, typically containing build
    /// reproduction steps.
    pub approval_notes: Option<String>,
    /// Optional source archive to attach to the version. AMO requires this
    /// when reviewers cannot reproduce the bundled artefact from the listing.
    pub source: Option<Vec<u8>>,
    /// Polling cadence and overall timeout used while waiting for AMO to
    /// finish validating the upload.
    pub poll: FirefoxPollConfig,
}

/// Polling cadence and budget for [`FirefoxStore::publish`]'s
/// validation-status loop.
///
/// Defaults to 1 second interval and 5 minute timeout.
#[derive(Debug, Clone)]
pub struct FirefoxPollConfig {
    /// Delay between successive polls of the upload status endpoint.
    pub interval: Duration,
    /// Maximum total time to wait before giving up with
    /// [`WepubError::Timeout`].
    pub timeout: Duration,
}

impl Default for FirefoxPollConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_POLL_TIMEOUT,
        }
    }
}

/// Distribution channel for an AMO version.
#[derive(Debug, Clone, Copy, Default)]
pub enum Channel {
    /// Listed on addons.mozilla.org. Goes through public review (the
    /// default).
    #[default]
    Listed,
    /// Self-distributed signed build. Reviewed but not listed.
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

/// Compatibility declaration sent to AMO when creating the version.
///
/// AMO's wire format accepts either a flat list of compatible apps (with
/// versions inferred from the manifest) or an object mapping each app to an
/// explicit version range. `wepub-core` exposes both shapes through this
/// enum.
#[derive(Debug, Clone)]
pub enum Compatibility {
    /// Shorthand form: list compatible apps; min/max come from the manifest.
    Apps(Vec<Application>),
    /// Detailed form: per-app explicit version range. An empty
    /// [`VersionRange`] means "use the value declared in the manifest".
    Detailed(HashMap<Application, VersionRange>),
}

impl Serialize for Compatibility {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        match self {
            Self::Apps(apps) => apps.serialize(serializer),
            Self::Detailed(map) => map.serialize(serializer),
        }
    }
}

/// AMO application identifier used in compatibility declarations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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

impl Serialize for Application {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

/// Explicit `min` / `max` application version pair used by
/// [`Compatibility::Detailed`].
///
/// Either bound can be `None`, in which case the corresponding manifest
/// value is used.
#[derive(Debug, Clone, Default, Serialize)]
pub struct VersionRange {
    /// Minimum compatible application version. `None` defers to the
    /// manifest's `strict_min_version`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    /// Maximum compatible application version. `None` defers to the
    /// manifest's `strict_max_version`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

/// Client for the AMO Add-on Versions API (v5).
///
/// The store holds the JWT credential pair and a reusable HTTP client; it
/// is cheap to construct and intended to live for the duration of a single
/// publish run.
// Debug intentionally omitted: holds the AMO JWT secret.
pub struct FirefoxStore {
    addon_id: String,
    issuer: String,
    secret: String,
    root_url: Url,
    client: reqwest::Client,
}

impl FirefoxStore {
    /// Build a store bound to `addon_id`, signing requests with the supplied
    /// HS256 JWT credential pair (issuer + secret).
    ///
    /// Get the credentials from
    /// <https://addons.mozilla.org/developers/addon/api/key/>.
    ///
    /// # Errors
    ///
    /// Fails if the underlying HTTP client cannot be built (e.g. rustls
    /// platform-verifier initialization fails).
    pub fn from_jwt_credentials(
        addon_id: String,
        jwt_issuer: String,
        jwt_secret: String,
    ) -> Result<Self> {
        Ok(Self {
            addon_id,
            issuer: jwt_issuer,
            secret: jwt_secret,
            root_url: Url::parse(DEFAULT_ROOT_URL).expect("DEFAULT_ROOT_URL is a valid URL"),
            client: build_client()?,
        })
    }

    /// Override the AMO API root URL.
    ///
    /// Defaults to `https://addons.mozilla.org/`. Intended for tests
    /// or when pointing at a local `mozilla/addons-server` instance. A
    /// missing trailing slash is added automatically so that relative paths
    /// join correctly.
    ///
    /// # Errors
    ///
    /// Returns [`WepubError::InvalidUrl`] if `root_url` does not parse as a
    /// URL.
    pub fn with_root_url(mut self, root_url: &str) -> Result<Self> {
        self.root_url = parse_root_url(root_url)?;
        Ok(self)
    }

    /// Upload `zip` and create a new version on the bound add-on.
    ///
    /// The call performs four steps internally: upload the archive, poll
    /// AMO until validation finishes, create the version, and (if
    /// `options.source` is set) attach the source archive in a follow-up
    /// PATCH. The polling cadence is controlled by `options.poll`.
    ///
    /// Progress (`uploading to Firefox Add-ons`, `polling Firefox Add-ons
    /// upload status`, `submitting to Firefox Add-ons for publish`,
    /// `Firefox Add-ons publish succeeded`) is emitted through the
    /// `tracing` crate; library consumers configure their own subscriber
    /// to render or capture it.
    ///
    /// # Errors
    ///
    /// On failure, returns one of [`WepubError::Network`],
    /// [`WepubError::HttpStatus`], [`WepubError::Timeout`],
    /// [`WepubError::UnexpectedResponse`],
    /// [`WepubError::FirefoxValidationFailed`], [`WepubError::Io`] or
    /// [`WepubError::Internal`] depending on which step failed.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// # async fn run() -> wepub_core::Result<()> {
    /// use wepub_core::firefox::{FirefoxStore, FirefoxPublishOptions};
    ///
    /// let store = FirefoxStore::from_jwt_credentials(
    ///     "myaddon@example.com".into(),
    ///     "user:12345:6789".into(),
    ///     "jwt-secret".into(),
    /// )?;
    /// let zip = std::fs::read("./addon.zip")?;
    /// store.publish(zip, FirefoxPublishOptions::default()).await?;
    /// # Ok(())
    /// # }
    /// ```
    pub async fn publish(&self, zip: Vec<u8>, options: FirefoxPublishOptions) -> Result<()> {
        let upload = self.upload(zip, options.channel).await?;
        let validated = self
            .wait_until_validated(&upload.uuid, &options.poll)
            .await?;

        let version = self
            .create_version(
                &validated.uuid,
                options.compatibility.as_ref(),
                &options.release_notes,
                options.approval_notes.as_deref(),
            )
            .await?;

        if let Some(source) = options.source {
            self.patch_version_source(version.id, source).await?;
        }

        tracing::info!(
            addon_id = %self.addon_id,
            version_id = version.id,
            "Firefox Add-ons publish succeeded"
        );
        Ok(())
    }

    fn endpoint(&self, path: &str) -> Result<Url> {
        join_endpoint(&self.root_url, path)
    }

    async fn upload(&self, zip: Vec<u8>, channel: Channel) -> Result<UploadResponse> {
        let method = reqwest::Method::POST;
        let url = self.endpoint("api/v5/addons/upload/")?;
        let auth = self.auth_header()?;

        let len = zip.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(zip), len)
            .file_name(UPLOAD_FILE_NAME)
            .mime_str("application/zip")
            .map_err(|e| WepubError::Internal(format!("invalid MIME literal: {e}")))?;
        let form = Form::new()
            .part("upload", part)
            .text("channel", channel.as_str());

        tracing::info!(
            addon_id = %self.addon_id,
            channel = channel.as_str(),
            "uploading to Firefox Add-ons"
        );

        log_request(&method, &url);
        let resp = self
            .client
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .await?;

        decode_response(resp, Store::Firefox, Phase::Upload).await
    }

    async fn wait_until_validated(
        &self,
        uuid: &str,
        config: &FirefoxPollConfig,
    ) -> Result<UploadResponse> {
        let url = self.endpoint(&format!("api/v5/addons/upload/{uuid}/"))?;
        let started = Instant::now();

        loop {
            let method = reqwest::Method::GET;
            let auth = self.auth_header()?;
            log_request(&method, &url);
            let resp = self
                .client
                .request(method, url.clone())
                .header(reqwest::header::AUTHORIZATION, auth)
                .send()
                .await?;
            let upload: UploadResponse =
                decode_response(resp, Store::Firefox, Phase::Upload).await?;

            tracing::info!(
                uuid = uuid,
                processed = upload.processed,
                valid = upload.valid,
                "polling Firefox Add-ons upload status"
            );

            if upload.processed {
                if upload.valid {
                    return Ok(upload);
                }
                let Some(validation) = upload.validation.as_ref() else {
                    return Err(WepubError::UnexpectedResponse {
                        store: Store::Firefox,
                        phase: Phase::Upload,
                        detail: "AMO reported valid=false without a validation field".to_string(),
                    });
                };
                let detail = pretty_json(validation);
                return Err(WepubError::FirefoxValidationFailed {
                    uuid: uuid.to_string(),
                    detail,
                });
            }

            if started.elapsed() >= config.timeout {
                return Err(WepubError::Timeout {
                    store: Store::Firefox,
                    phase: Phase::Upload,
                    elapsed: config.timeout,
                });
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    async fn create_version(
        &self,
        upload_uuid: &str,
        compatibility: Option<&Compatibility>,
        release_notes: &HashMap<String, String>,
        approval_notes: Option<&str>,
    ) -> Result<VersionResponse> {
        let method = reqwest::Method::POST;
        let url = self.endpoint(&format!("api/v5/addons/addon/{}/versions/", self.addon_id))?;
        let auth = self.auth_header()?;

        let body = VersionCreateBody {
            upload: upload_uuid,
            compatibility,
            release_notes,
            approval_notes,
        };

        tracing::info!(
            addon_id = %self.addon_id,
            uuid = upload_uuid,
            "submitting to Firefox Add-ons for publish"
        );

        log_request(&method, &url);
        let resp = self
            .client
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&body)
            .send()
            .await?;

        decode_response(resp, Store::Firefox, Phase::Publish).await
    }

    async fn patch_version_source(
        &self,
        version_id: u64,
        source: Vec<u8>,
    ) -> Result<VersionResponse> {
        let method = reqwest::Method::PATCH;
        let url = self.endpoint(&format!(
            "api/v5/addons/addon/{}/versions/{version_id}/",
            self.addon_id
        ))?;
        let auth = self.auth_header()?;

        let len = source.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(source), len)
            .file_name("source.zip")
            .mime_str("application/zip")
            .map_err(|e| WepubError::Internal(format!("invalid MIME literal: {e}")))?;
        let form = Form::new().part("source", part);

        tracing::info!(
            addon_id = %self.addon_id,
            version_id,
            "uploading source to Firefox Add-ons"
        );

        log_request(&method, &url);
        let resp = self
            .client
            .request(method, url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .await?;

        decode_response(resp, Store::Firefox, Phase::Publish).await
    }

    fn auth_header(&self) -> Result<String> {
        let token = generate_jwt(&self.issuer, &self.secret)?;
        Ok(format!("JWT {token}"))
    }
}

// Successful response from creating a new add-on version on AMO.
// Internal-only: the id is echoed via `tracing::info!` from `publish` and
// not surfaced to the caller because the only documented use was logging.
#[derive(Debug, Clone, Deserialize)]
struct VersionResponse {
    id: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    uuid: String,
    processed: bool,
    valid: bool,
    #[serde(default)]
    validation: Option<serde_json::Value>,
}

#[derive(Serialize)]
struct VersionCreateBody<'a> {
    upload: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    compatibility: Option<&'a Compatibility>,
    #[serde(skip_serializing_if = "HashMap::is_empty")]
    release_notes: &'a HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    approval_notes: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header_exists, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn upload_posts_multipart_and_parses_response() {
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

        let store = store_for(&server);
        let resp = store
            .upload(b"fake-zip".to_vec(), Channel::Listed)
            .await
            .unwrap();

        assert_eq!(resp.uuid, "abc-123");
        assert!(!resp.processed);
    }

    #[tokio::test]
    async fn wait_until_validated_returns_when_processed_and_valid() {
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

        let store = store_for(&server);
        let resp = store
            .wait_until_validated("uuid-1", &fast_poll())
            .await
            .unwrap();

        assert_eq!(resp.uuid, "uuid-1");
        assert!(resp.processed);
        assert!(resp.valid);
    }

    #[tokio::test]
    async fn wait_until_validated_errors_on_invalid_validation() {
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

        let store = store_for(&server);
        let err = store
            .wait_until_validated("uuid-2", &fast_poll())
            .await
            .unwrap_err();

        match err {
            WepubError::FirefoxValidationFailed { uuid, detail } => {
                assert_eq!(uuid, "uuid-2");
                assert!(detail.contains("manifest broken"));
            }
            other => panic!("expected WepubError::FirefoxValidationFailed, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_validated_times_out_when_processing_never_completes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-3/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-3", false, false)),
            )
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_validated("uuid-3", &fast_poll())
            .await
            .unwrap_err();

        match err {
            WepubError::Timeout { .. } => {}
            other => panic!("expected WepubError::Timeout, got {other:?}"),
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

        let store = store_for(&server);
        let resp = store
            .create_version("uuid-x", None, &HashMap::new(), None)
            .await
            .unwrap();

        assert_eq!(resp.id, 4242);
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

        let store = store_for(&server);
        let resp = store
            .patch_version_source(4242, b"source-zip".to_vec())
            .await
            .unwrap();

        assert_eq!(resp.id, 4242);
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

        let store = store_for(&server);
        let options = FirefoxPublishOptions {
            source: Some(b"source-zip".to_vec()),
            poll: fast_poll(),
            ..FirefoxPublishOptions::default()
        };
        store.publish(b"zip".to_vec(), options).await.unwrap();
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

        Mock::given(method("GET"))
            .and(path("/api/v5/addons/upload/uuid-ns/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-ns", true, true)),
            )
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

        let store = store_for(&server);
        let options = FirefoxPublishOptions {
            poll: fast_poll(),
            ..FirefoxPublishOptions::default()
        };
        store.publish(b"zip".to_vec(), options).await.unwrap();
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

        let store = store_for(&server);
        let options = FirefoxPublishOptions {
            poll: fast_poll(),
            ..FirefoxPublishOptions::default()
        };
        let err = store.publish(b"zip".to_vec(), options).await.unwrap_err();

        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 401);
                assert_eq!(body, "unauthorized");
            }
            other => panic!("expected WepubError::HttpStatus, got {other:?}"),
        }
    }

    // ---- Unit tests for serialization and URL helpers ----

    #[test]
    fn channel_serialises_as_amo_expects() {
        assert_eq!(Channel::Listed.as_str(), "listed");
        assert_eq!(Channel::Unlisted.as_str(), "unlisted");
    }

    #[test]
    fn version_create_body_minimal_only_has_upload() {
        let json = body_to_json("uuid-123", None, &HashMap::new(), None);
        assert_eq!(json, serde_json::json!({ "upload": "uuid-123" }));
    }

    #[test]
    fn version_create_body_with_apps_shorthand() {
        let compat = Compatibility::Apps(vec![Application::Firefox, Application::Android]);
        let json = body_to_json("uuid-123", Some(&compat), &HashMap::new(), None);
        assert_eq!(
            json,
            serde_json::json!({
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
        let compat = Compatibility::Detailed(map);
        let json = body_to_json("uuid-123", Some(&compat), &HashMap::new(), None);

        assert_eq!(json["upload"], "uuid-123");
        assert_eq!(
            json["compatibility"]["firefox"],
            serde_json::json!({ "min": "58.0", "max": "120.0" })
        );
        assert_eq!(
            json["compatibility"]["android"],
            serde_json::json!({ "min": "58.0" })
        );
    }

    #[test]
    fn version_create_body_includes_release_notes_and_approval_notes() {
        let mut notes = HashMap::new();
        notes.insert("en-US".into(), "Hello".into());
        notes.insert("ja".into(), "こんにちは".into());

        let json = body_to_json("uuid-123", None, &notes, Some("for reviewers"));

        assert_eq!(json["upload"], "uuid-123");
        assert_eq!(json["release_notes"]["en-US"], "Hello");
        assert_eq!(json["release_notes"]["ja"], "こんにちは");
        assert_eq!(json["approval_notes"], "for reviewers");
    }

    #[test]
    fn endpoint_joins_relative_path() {
        let store = FirefoxStore::from_jwt_credentials(
            "test-addon".into(),
            "issuer".into(),
            "secret".into(),
        )
        .unwrap();
        let url = store.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(
            url.as_str(),
            "https://addons.mozilla.org/api/v5/addons/upload/"
        );
    }

    #[test]
    fn with_root_url_overrides_default() {
        let store = FirefoxStore::from_jwt_credentials(
            "test-addon".into(),
            "issuer".into(),
            "secret".into(),
        )
        .unwrap()
        .with_root_url("http://127.0.0.1:8000/")
        .unwrap();
        let url = store.endpoint("api/v5/addons/upload/").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8000/api/v5/addons/upload/");
    }

    #[test]
    fn with_root_url_rejects_garbage() {
        let store = FirefoxStore::from_jwt_credentials(
            "test-addon".into(),
            "issuer".into(),
            "secret".into(),
        )
        .unwrap();
        let Err(err) = store.with_root_url("not a url") else {
            panic!("expected with_root_url to reject");
        };
        assert!(matches!(err, WepubError::InvalidUrl(_)), "got {err:?}");
    }

    fn store_for(server: &MockServer) -> FirefoxStore {
        FirefoxStore::from_jwt_credentials("test-addon".into(), "issuer".into(), "secret".into())
            .unwrap()
            .with_root_url(&server.uri())
            .unwrap()
    }

    fn fast_poll() -> FirefoxPollConfig {
        FirefoxPollConfig {
            interval: Duration::from_millis(10),
            timeout: Duration::from_millis(200),
        }
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
        upload: &str,
        compatibility: Option<&Compatibility>,
        release_notes: &HashMap<String, String>,
        approval_notes: Option<&str>,
    ) -> serde_json::Value {
        serde_json::to_value(VersionCreateBody {
            upload,
            compatibility,
            release_notes,
            approval_notes,
        })
        .unwrap()
    }
}
