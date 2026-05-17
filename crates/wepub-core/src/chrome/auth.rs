use serde::Deserialize;

use crate::{Phase, Result, Store, WepubError};

pub(crate) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

pub(crate) async fn refresh_access_token(
    client: &reqwest::Client,
    token_url: &str,
    client_id: &str,
    client_secret: &str,
    refresh_token: &str,
) -> Result<String> {
    let response = client
        .post(token_url)
        .form(&[
            ("grant_type", "refresh_token"),
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("refresh_token", refresh_token),
        ])
        .send()
        .await?;

    let status = response.status();
    let body = response.text().await?;
    tracing::debug!(
        store = %Store::Chrome,
        phase = %Phase::TokenRefresh,
        status = status.as_u16(),
        body = %body,
        "received OAuth token endpoint response",
    );

    if !status.is_success() {
        return Err(WepubError::HttpStatus {
            status: status.as_u16(),
            body,
        });
    }

    let parsed: TokenResponse =
        serde_json::from_str(&body).map_err(|e| WepubError::UnexpectedResponse {
            store: Store::Chrome,
            phase: Phase::TokenRefresh,
            detail: format!("failed to decode token response: {e}"),
        })?;
    Ok(parsed.access_token)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{body_string_contains, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn refresh(server: &MockServer, secret: &str) -> Result<String> {
        let client = reqwest::Client::new();
        refresh_access_token(
            &client,
            &server.uri(),
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
    async fn http_error_preserves_response_body_verbatim() {
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
            WepubError::HttpStatus { status, body } => {
                assert_eq!(status, 400);
                assert!(body.contains("invalid_grant"), "body: {body}");
                assert!(
                    body.contains("Token has been expired or revoked."),
                    "body: {body}",
                );
            }
            other => panic!("expected HttpStatus, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_error_body_is_passed_through() {
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
            WepubError::UnexpectedResponse {
                store,
                phase,
                detail,
            } => {
                assert_eq!(store, Store::Chrome);
                assert_eq!(phase, Phase::TokenRefresh);
                assert!(
                    detail.contains("access_token"),
                    "expected detail to mention the missing field, got: {detail}",
                );
            }
            other => panic!("expected UnexpectedResponse, got {other:?}"),
        }
    }
}
