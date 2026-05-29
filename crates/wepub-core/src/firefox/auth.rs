use std::time::{SystemTime, UNIX_EPOCH};

use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde::Serialize;
use uuid::Uuid;

const TOKEN_LIFETIME_SECS: u64 = 60;

pub(crate) fn generate_jwt(issuer: &str, secret: &str) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is set after the UNIX epoch")
        .as_secs();

    let claims = Claims {
        iss: issuer.to_string(),
        jti: Uuid::new_v4().to_string(),
        iat: now,
        exp: now + TOKEN_LIFETIME_SECS,
    };

    encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("HS256 JWT encoding cannot fail for valid claims")
}

#[derive(Serialize)]
struct Claims {
    iss: String,
    jti: String,
    iat: u64,
    exp: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{DecodingKey, Validation, decode};
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    struct DecodedClaims {
        iss: String,
        jti: String,
        iat: u64,
        exp: u64,
    }

    #[test]
    fn jwt_payload_matches_amo_spec() {
        let token = generate_jwt("user:123:456", "secret-key");

        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();

        let decoded = decode::<DecodedClaims>(
            &token,
            &DecodingKey::from_secret(b"secret-key"),
            &validation,
        )
        .unwrap();

        assert_eq!(decoded.claims.iss, "user:123:456");
        assert_eq!(decoded.claims.exp - decoded.claims.iat, TOKEN_LIFETIME_SECS);
        assert_eq!(decoded.claims.jti.len(), 36); // UUID v4 string length
    }

    #[test]
    fn jwt_rejects_wrong_secret() {
        let token = generate_jwt("issuer", "correct-secret");

        let mut validation = Validation::new(Algorithm::HS256);
        validation.validate_exp = false;
        validation.required_spec_claims.clear();

        let err = decode::<DecodedClaims>(
            &token,
            &DecodingKey::from_secret(b"wrong-secret"),
            &validation,
        )
        .unwrap_err();

        assert!(matches!(
            err.kind(),
            jsonwebtoken::errors::ErrorKind::InvalidSignature
        ));
    }

    #[test]
    fn each_call_generates_unique_jti() {
        let t1 = generate_jwt("issuer", "secret");
        let t2 = generate_jwt("issuer", "secret");
        assert_ne!(t1, t2, "jti should be unique per call");
    }
}
