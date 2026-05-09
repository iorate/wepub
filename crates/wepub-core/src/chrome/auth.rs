use serde::Deserialize;

use crate::{Result, WepubError};

pub(crate) const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct TokenErrorResponse {
    error: String,
    error_description: Option<String>,
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

    if status.is_success() {
        let parsed: TokenResponse = serde_json::from_str(&body)?;
        Ok(parsed.access_token)
    } else {
        Err(parse_token_error(&body))
    }
}

fn parse_token_error(body: &str) -> WepubError {
    let Ok(err) = serde_json::from_str::<TokenErrorResponse>(body) else {
        return WepubError::Auth(format!("token endpoint returned non-JSON error: {body}"));
    };

    let message = match err.error_description {
        Some(desc) => format!("{}: {desc}", err.error),
        None => err.error,
    };
    WepubError::Auth(message)
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
    async fn error_with_description_formats_as_pair() {
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
            WepubError::Auth(msg) => {
                assert_eq!(msg, "invalid_grant: Token has been expired or revoked.");
            }
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn error_without_description_uses_code_only() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(json!({ "error": "invalid_request" })),
            )
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::Auth(msg) => assert_eq!(msg, "invalid_request"),
            other => panic!("expected Auth, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn non_json_error_body_falls_back_to_raw_text() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(503).set_body_string("<html>Service Unavailable</html>"),
            )
            .mount(&server)
            .await;

        let err = refresh(&server, "client-secret").await.unwrap_err();
        match err {
            WepubError::Auth(msg) => {
                assert!(
                    msg.contains("<html>Service Unavailable</html>"),
                    "raw body should be preserved, got: {msg}"
                );
            }
            other => panic!("expected Auth, got {other:?}"),
        }
    }
}
