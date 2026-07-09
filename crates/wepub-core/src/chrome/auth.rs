use serde::Deserialize;
use tracing::debug;
use url::Url;

use crate::{Result, WepubError, common::send_request};

pub(crate) const DEFAULT_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

pub(crate) async fn refresh_access_token(
    client: &reqwest::Client,
    token_url: Url,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String> {
    let req = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
        ])
        .build()
        .map_err(WepubError::http)?;

    let resp = send_request(client, req).await?;

    let status = resp.status();
    let body = resp.text().await.map_err(WepubError::http)?;
    if !status.is_success() {
        debug!(
            status = status.as_u16(),
            body = body.as_str(),
            "received response"
        );
        if (status == 400 || status == 401)
            && let Ok(token_error) = serde_json::from_str::<TokenErrorResponse>(&body)
        {
            return Err(WepubError::OAuthToken {
                error: token_error.error,
                error_description: token_error.error_description,
                error_uri: token_error.error_uri,
            });
        }
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }
    // The success body carries the access token, so mask it in logs.
    debug!(status = status.as_u16(), body = "***", "received response");
    let token: TokenResponse =
        serde_json::from_str(&body).map_err(|err| WepubError::UnexpectedResponse {
            reason: format!("failed to decode response: {err}"),
        })?;
    Ok(token.access_token)
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
    error_description: Option<String>,
    error_uri: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn refresh(server: &MockServer, secret: &str) -> Result<String> {
        let client = reqwest::Client::new();
        let token_url = Url::parse(&server.uri()).unwrap();
        refresh_access_token(
            &client,
            token_url,
            "client-id",
            secret,
            "refresh-token-value",
        )
        .await
    }

    #[tokio::test]
    async fn returns_access_token_on_success() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "ya29.example",
                "expires_in": 3599,
                "token_type": "Bearer",
                "scope": "https://www.googleapis.com/auth/chromewebstore",
            })))
            .expect(1)
            .mount(&server)
            .await;

        let token = refresh(&server, "client-secret").await.unwrap();
        assert_eq!(token, "ya29.example");
    }

    #[tokio::test]
    async fn sends_form_encoded_body_with_required_fields() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/"))
            .and(header("content-type", "application/x-www-form-urlencoded"))
            .and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("client_id=client-id"))
            .and(body_string_contains("client_secret=client-secret"))
            .and(body_string_contains("refresh_token=refresh-token-value"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({ "access_token": "tok" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        refresh(&server, "client-secret").await.unwrap();
    }

    #[tokio::test]
    async fn status_400_parses_into_oauth_token_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "error": "invalid_grant",
                "error_description": "Token has been expired or revoked.",
            })))
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::OAuthToken {
                error,
                error_description,
                ..
            } => {
                assert_eq!(error, "invalid_grant");
                assert_eq!(
                    error_description.unwrap(),
                    "Token has been expired or revoked."
                );
            }
            other => panic!("expected OAuthToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_401_parses_into_oauth_token_error() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({
                "error": "invalid_client",
            })))
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::OAuthToken { error, .. } => {
                assert_eq!(error, "invalid_client");
            }
            other => panic!("expected OAuthToken, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn status_400_with_unparseable_body_falls_back_to_http_status() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_string("<html>Bad Request</html>"))
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("<html>Bad Request</html>"), "got: {body}");
            }
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_400_status_is_passed_through() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(503).set_body_string("<html>Service Unavailable</html>"),
            )
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 503);
                assert!(
                    body.contains("<html>Service Unavailable</html>"),
                    "raw body should be preserved, got: {body}",
                );
            }
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn success_without_access_token_field_becomes_unexpected_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "expires_in": 3599 })))
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::UnexpectedResponse { reason } => {
                assert!(
                    reason.contains("access_token"),
                    "expected reason to mention the missing field, got: {reason}",
                );
            }
            other => panic!("expected UnexpectedResponse, got {other:?}"),
        }
    }
}
