use std::collections::HashMap;
use std::time::{Duration, Instant};

use reqwest::multipart::{Form, Part};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{Result, WepubError, http::build_client};

use super::auth::generate_jwt;

const DEFAULT_BASE_URL: &str = "https://addons.mozilla.org/api/v5/";
const UPLOAD_FILE_NAME: &str = "addon.zip";
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_POLL_TIMEOUT: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Default)]
pub struct PublishOptions {
    pub channel: Channel,
    pub compatibility: Option<Compatibility>,
    pub release_notes: HashMap<String, String>,
    pub approval_notes: Option<String>,
    pub source: Option<Vec<u8>>,
    pub poll: PollConfig,
}

#[derive(Debug, Clone)]
pub struct PollConfig {
    pub interval: Duration,
    pub timeout: Duration,
}

impl Default for PollConfig {
    fn default() -> Self {
        Self {
            interval: DEFAULT_POLL_INTERVAL,
            timeout: DEFAULT_POLL_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub enum Channel {
    #[default]
    Listed,
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

#[derive(Debug, Clone)]
pub enum Compatibility {
    /// Shorthand: list compatible apps; min/max come from the manifest.
    Apps(Vec<Application>),
    /// Detailed: per-app version range. Empty `VersionRange` means "use manifest".
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Application {
    Firefox,
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

#[derive(Debug, Clone, Default, Serialize)]
pub struct VersionRange {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VersionResponse {
    pub id: u64,
}

// Debug intentionally omitted: holds the AMO JWT secret.
pub struct FirefoxStore {
    addon_id: String,
    issuer: String,
    secret: String,
    base_url: Url,
    client: reqwest::Client,
}

impl FirefoxStore {
    pub fn from_jwt_credentials(
        addon_id: String,
        jwt_issuer: String,
        jwt_secret: String,
    ) -> Result<Self> {
        Ok(Self {
            addon_id,
            issuer: jwt_issuer,
            secret: jwt_secret,
            base_url: Url::parse(DEFAULT_BASE_URL).expect("DEFAULT_BASE_URL is a valid URL"),
            client: build_client()?,
        })
    }

    #[must_use]
    pub fn with_base_url(mut self, base_url: Url) -> Self {
        self.base_url = ensure_trailing_slash(base_url);
        self
    }

    pub async fn publish(&self, zip: Vec<u8>, options: PublishOptions) -> Result<VersionResponse> {
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

        Ok(version)
    }

    pub(crate) fn endpoint(&self, path: &str) -> Result<Url> {
        self.base_url
            .join(path)
            .map_err(|e| WepubError::Internal(format!("invalid endpoint path {path:?}: {e}")))
    }

    pub(crate) async fn upload(&self, zip: Vec<u8>, channel: Channel) -> Result<UploadResponse> {
        let url = self.endpoint("addons/upload/")?;
        let auth = self.auth_header()?;

        let len = zip.len() as u64;
        let part = Part::stream_with_length(reqwest::Body::from(zip), len)
            .file_name(UPLOAD_FILE_NAME)
            .mime_str("application/zip")
            .map_err(|e| WepubError::Internal(format!("invalid MIME literal: {e}")))?;
        let form = Form::new()
            .part("upload", part)
            .text("channel", channel.as_str());

        tracing::info!(addon_id = %self.addon_id, channel = channel.as_str(), "uploading add-on to AMO");

        let resp = self
            .client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .await?;

        decode_response(resp).await
    }

    pub(crate) async fn wait_until_validated(
        &self,
        uuid: &str,
        config: &PollConfig,
    ) -> Result<UploadResponse> {
        let url = self.endpoint(&format!("addons/upload/{uuid}/"))?;
        let started = Instant::now();

        loop {
            let auth = self.auth_header()?;
            let resp = self
                .client
                .get(url.clone())
                .header(reqwest::header::AUTHORIZATION, auth)
                .send()
                .await?;
            let upload: UploadResponse = decode_response(resp).await?;

            tracing::info!(
                uuid = uuid,
                processed = upload.processed,
                valid = upload.valid,
                "polling AMO upload status"
            );

            if upload.processed {
                if upload.valid {
                    return Ok(upload);
                }
                let body = upload.validation.as_ref().map_or_else(
                    || "validation failed (no detail provided)".to_string(),
                    |v| serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()),
                );
                return Err(WepubError::Validation(body));
            }

            if started.elapsed() >= config.timeout {
                return Err(WepubError::Validation(format!(
                    "validation timed out after {:?}",
                    config.timeout
                )));
            }

            tokio::time::sleep(config.interval).await;
        }
    }

    pub(crate) async fn create_version(
        &self,
        upload_uuid: &str,
        compatibility: Option<&Compatibility>,
        release_notes: &HashMap<String, String>,
        approval_notes: Option<&str>,
    ) -> Result<VersionResponse> {
        let url = self.endpoint(&format!("addons/addon/{}/versions/", self.addon_id))?;
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
            "creating AMO version"
        );

        let resp = self
            .client
            .post(url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .json(&body)
            .send()
            .await?;

        decode_response(resp).await
    }

    pub(crate) async fn patch_version_source(
        &self,
        version_id: u64,
        source: Vec<u8>,
    ) -> Result<VersionResponse> {
        let url = self.endpoint(&format!(
            "addons/addon/{}/versions/{version_id}/",
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
            "uploading version source to AMO"
        );

        let resp = self
            .client
            .patch(url)
            .header(reqwest::header::AUTHORIZATION, auth)
            .multipart(form)
            .send()
            .await?;

        decode_response(resp).await
    }

    fn auth_header(&self) -> Result<String> {
        let token = generate_jwt(&self.issuer, &self.secret)?;
        Ok(format!("JWT {token}"))
    }
}

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct UploadResponse {
    pub uuid: String,
    pub processed: bool,
    pub valid: bool,
    #[serde(default)]
    pub validation: Option<serde_json::Value>,
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

async fn decode_response<T: serde::de::DeserializeOwned>(resp: reqwest::Response) -> Result<T> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await?;
        return Err(WepubError::Api {
            status: status.as_u16(),
            body,
        });
    }
    let body = resp.bytes().await?;
    serde_json::from_slice(&body).map_err(WepubError::from)
}

fn ensure_trailing_slash(mut url: Url) -> Url {
    if !url.path().ends_with('/') {
        let new_path = format!("{}/", url.path());
        url.set_path(&new_path);
    }
    url
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
            .and(path("/addons/upload/"))
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
            .and(path("/addons/upload/uuid-1/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-1", false, false)),
            )
            .up_to_n_times(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/addons/upload/uuid-1/"))
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
            .and(path("/addons/upload/uuid-2/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;

        let store = store_for(&server);
        let err = store
            .wait_until_validated("uuid-2", &fast_poll())
            .await
            .unwrap_err();

        match err {
            WepubError::Validation(body) => {
                assert!(body.contains("manifest broken"));
            }
            other => panic!("expected WepubError::Validation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn wait_until_validated_times_out_when_processing_never_completes() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/addons/upload/uuid-3/"))
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
            WepubError::Validation(body) => {
                assert!(body.contains("timed out"));
            }
            other => panic!("expected WepubError::Validation, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_version_posts_json_and_parses_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/addons/addon/test-addon/versions/"))
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
            .and(path("/addons/addon/test-addon/versions/4242/"))
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
            .and(path("/addons/upload/"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("uuid-pub", false, false)),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/addons/upload/uuid-pub/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-pub", true, true)),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 7777 })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/addons/addon/test-addon/versions/7777/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 7777 })))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let options = PublishOptions {
            source: Some(b"source-zip".to_vec()),
            poll: fast_poll(),
            ..PublishOptions::default()
        };
        let resp = store.publish(b"zip".to_vec(), options).await.unwrap();

        assert_eq!(resp.id, 7777);
    }

    #[tokio::test]
    async fn publish_skips_source_patch_when_no_source_provided() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/addons/upload/"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(upload_json("uuid-ns", true, true)),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/addons/upload/uuid-ns/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(upload_json("uuid-ns", true, true)),
            )
            .mount(&server)
            .await;

        Mock::given(method("POST"))
            .and(path("/addons/addon/test-addon/versions/"))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({ "id": 9999 })))
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("PATCH"))
            .and(path("/addons/addon/test-addon/versions/9999/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "id": 9999 })))
            .expect(0)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let options = PublishOptions {
            poll: fast_poll(),
            ..PublishOptions::default()
        };
        let resp = store.publish(b"zip".to_vec(), options).await.unwrap();

        assert_eq!(resp.id, 9999);
    }

    #[tokio::test]
    async fn publish_propagates_upload_api_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/addons/upload/"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .expect(1)
            .mount(&server)
            .await;

        let store = store_for(&server);
        let options = PublishOptions {
            poll: fast_poll(),
            ..PublishOptions::default()
        };
        let err = store.publish(b"zip".to_vec(), options).await.unwrap_err();

        match err {
            WepubError::Api { status, body } => {
                assert_eq!(status, 401);
                assert_eq!(body, "unauthorized");
            }
            other => panic!("expected WepubError::Api, got {other:?}"),
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
        let url = store.endpoint("addons/upload/").unwrap();
        assert_eq!(
            url.as_str(),
            "https://addons.mozilla.org/api/v5/addons/upload/"
        );
    }

    #[test]
    fn with_base_url_overrides_default() {
        let store = FirefoxStore::from_jwt_credentials(
            "test-addon".into(),
            "issuer".into(),
            "secret".into(),
        )
        .unwrap()
        .with_base_url(Url::parse("http://127.0.0.1:8000/api/v5/").unwrap());
        let url = store.endpoint("addons/upload/").unwrap();
        assert_eq!(url.as_str(), "http://127.0.0.1:8000/api/v5/addons/upload/");
    }

    #[test]
    fn ensure_trailing_slash_appends_when_missing() {
        let url = Url::parse("https://example.com/api/v5").unwrap();
        let result = ensure_trailing_slash(url);
        assert_eq!(result.as_str(), "https://example.com/api/v5/");
    }

    #[test]
    fn ensure_trailing_slash_is_idempotent() {
        let url = Url::parse("https://example.com/api/v5/").unwrap();
        let result = ensure_trailing_slash(url);
        assert_eq!(result.as_str(), "https://example.com/api/v5/");
    }

    fn store_for(server: &MockServer) -> FirefoxStore {
        FirefoxStore::from_jwt_credentials("test-addon".into(), "issuer".into(), "secret".into())
            .unwrap()
            .with_base_url(Url::parse(&server.uri()).unwrap())
    }

    fn fast_poll() -> PollConfig {
        PollConfig {
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
