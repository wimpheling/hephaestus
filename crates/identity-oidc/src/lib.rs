//! OIDC signature and claim validation.

use identity_application::VerifiedExternalIdentity;
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Verified standard and provider OIDC claims.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedOidcClaims {
    /// Token issuer.
    pub iss: String,
    /// Stable issuer-local subject.
    pub sub: String,
    /// Intended audiences.
    pub aud: Audience,
    /// Expiry as a Unix timestamp.
    pub exp: u64,
    /// Issued-at time when supplied.
    pub iat: Option<u64>,
    /// Nonce for an interactive authorization flow.
    pub nonce: Option<String>,
    /// All additional provider claims.
    #[serde(flatten)]
    pub provider: serde_json::Map<String, Value>,
}

/// One or multiple token audiences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Audience {
    /// One audience.
    One(String),
    /// Multiple audiences.
    Many(Vec<String>),
}

/// Configured verifier for one trusted OIDC issuer and signing key.
pub struct OidcVerifier {
    issuer: String,
    validation: Validation,
    decoding_key: DecodingKey,
}

impl OidcVerifier {
    /// Creates a verifier for a key already resolved from the issuer's trusted
    /// JWKS document.
    #[must_use]
    pub fn new(
        issuer: impl Into<String>,
        audience: &str,
        algorithm: Algorithm,
        decoding_key: DecodingKey,
    ) -> Self {
        let issuer = issuer.into();
        let mut validation = Validation::new(algorithm);
        validation.set_issuer(&[issuer.as_str()]);
        validation.set_audience(&[audience]);
        validation.set_required_spec_claims(&["exp", "iss", "sub", "aud"]);
        validation.validate_exp = true;
        Self {
            issuer,
            validation,
            decoding_key,
        }
    }

    /// Verifies signature, issuer, audience, expiry, and an expected nonce.
    ///
    /// Pass `None` for non-interactive bearer-token flows where no nonce was
    /// issued. Authorization-code middleware must supply the session nonce.
    ///
    /// # Errors
    ///
    /// Returns a typed failure for any signature or claim mismatch.
    pub fn verify(
        &self,
        token: &str,
        expected_nonce: Option<&str>,
    ) -> Result<VerifiedExternalIdentity, OidcError> {
        let token = decode::<VerifiedOidcClaims>(token, &self.decoding_key, &self.validation)
            .map_err(OidcError::InvalidToken)?;
        if token.claims.iss != self.issuer {
            return Err(OidcError::IssuerMismatch);
        }
        if let Some(expected) = expected_nonce {
            if token.claims.nonce.as_deref() != Some(expected) {
                return Err(OidcError::NonceMismatch);
            }
        }
        let claims = serde_json::to_value(&token.claims).map_err(OidcError::Claims)?;
        Ok(VerifiedExternalIdentity {
            issuer: token.claims.iss,
            subject: token.claims.sub,
            claims,
        })
    }
}

/// Validates the state returned by an interactive authorization response.
///
/// The expected value must come from the same server-side session that
/// initiated the authorization request.
///
/// # Errors
///
/// Returns [`OidcError::StateMismatch`] when the values differ.
pub fn validate_authorization_state(expected: &str, received: &str) -> Result<(), OidcError> {
    if expected == received {
        Ok(())
    } else {
        Err(OidcError::StateMismatch)
    }
}

/// OIDC verification failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum OidcError {
    /// JWT signature or registered claims are invalid.
    #[error("OIDC token is invalid")]
    InvalidToken(#[source] jsonwebtoken::errors::Error),
    /// Issuer did not match the configured provider.
    #[error("OIDC issuer does not match")]
    IssuerMismatch,
    /// Interactive-flow nonce did not match the session.
    #[error("OIDC nonce does not match")]
    NonceMismatch,
    /// Interactive-flow state did not match the initiating session.
    #[error("OIDC authorization state does not match")]
    StateMismatch,
    /// Validated claims could not be represented as JSON.
    #[error("OIDC claims could not be represented")]
    Claims(#[source] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use super::{OidcError, OidcVerifier, validate_authorization_state};
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};
    use serde_json::json;
    use std::collections::BTreeMap;

    const SECRET: &[u8] = b"development-test-secret-with-sufficient-length";

    fn token(issuer: &str, audience: &str, nonce: &str, expiry: u64) -> String {
        encode(
            &Header::new(Algorithm::HS256),
            &json!({
                "iss": issuer,
                "sub": "provider-subject",
                "aud": audience,
                "exp": expiry,
                "nonce": nonce,
                "email_verified": true,
                "groups": BTreeMap::<String, String>::new()
            }),
            &EncodingKey::from_secret(SECRET),
        )
        .expect("encode test token")
    }

    #[test]
    fn verifies_all_registered_claims_and_nonce() {
        let verifier = OidcVerifier::new(
            "https://issuer.example",
            "hephaestus",
            Algorithm::HS256,
            DecodingKey::from_secret(SECRET),
        );
        let expiry = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system time")
            .as_secs()
            + 300;
        let valid = token(
            "https://issuer.example",
            "hephaestus",
            "session-nonce",
            expiry,
        );
        let identity = verifier
            .verify(&valid, Some("session-nonce"))
            .expect("valid token");
        assert_eq!(identity.subject, "provider-subject");
        assert!(matches!(
            verifier.verify(&valid, Some("wrong")),
            Err(OidcError::NonceMismatch)
        ));

        let wrong_audience = token(
            "https://issuer.example",
            "another-service",
            "session-nonce",
            expiry,
        );
        assert!(matches!(
            verifier.verify(&wrong_audience, Some("session-nonce")),
            Err(OidcError::InvalidToken(_))
        ));
        let expired = token("https://issuer.example", "hephaestus", "session-nonce", 1);
        assert!(matches!(
            verifier.verify(&expired, Some("session-nonce")),
            Err(OidcError::InvalidToken(_))
        ));
        assert!(validate_authorization_state("state", "state").is_ok());
        assert!(matches!(
            validate_authorization_state("state", "attacker"),
            Err(OidcError::StateMismatch)
        ));
    }
}
