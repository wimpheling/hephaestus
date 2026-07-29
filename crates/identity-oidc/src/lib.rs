//! OIDC signature and claim validation followed by internal identity mapping.

use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::{Postgres, Transaction};

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

/// Result of cryptographic OIDC verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedOidcIdentity {
    /// Verified issuer.
    pub issuer: String,
    /// Verified issuer-local subject.
    pub subject: String,
    /// Complete validated claims persisted in the user profile.
    pub claims: Value,
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
    ) -> Result<VerifiedOidcIdentity, OidcError> {
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
        Ok(VerifiedOidcIdentity {
            issuer: token.claims.iss,
            subject: token.claims.sub,
            claims,
        })
    }
}

/// Maps a verified issuer/subject pair to exactly one active internal user and
/// refreshes its validated profile in the same transaction.
///
/// # Errors
///
/// Returns a typed error for an unmapped or inactive identity or database
/// failure.
pub async fn map_identity(
    transaction: &mut Transaction<'_, Postgres>,
    verified: &VerifiedOidcIdentity,
    request_id: RequestId,
    trace_id: Option<&str>,
) -> Result<AuthenticatedIdentity, OidcError> {
    let row: Option<(uuid::Uuid, String)> = sqlx::query_as(
        "SELECT users.id, users.status
         FROM external_identities
         JOIN users ON users.id = external_identities.user_id
         WHERE external_identities.issuer = $1
           AND external_identities.subject = $2",
    )
    .bind(&verified.issuer)
    .bind(&verified.subject)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(OidcError::Database)?;
    let Some((user_id, status)) = row else {
        return Err(OidcError::UnmappedIdentity);
    };
    if status != "active" {
        return Err(OidcError::InactiveUser);
    }
    sqlx::query(
        "SELECT set_config('hephaestus.actor_id', $1, true),
                set_config('hephaestus.subject_type', 'user', true),
                set_config('hephaestus.request_id', $2, true)",
    )
    .bind(user_id.to_string())
    .bind(request_id.to_string())
    .execute(&mut **transaction)
    .await
    .map_err(OidcError::Database)?;
    sqlx::query(
        "INSERT INTO user_profiles (user_id, validated_claims)
         VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE
         SET validated_claims = EXCLUDED.validated_claims, updated_at = now()",
    )
    .bind(user_id)
    .bind(&verified.claims)
    .execute(&mut **transaction)
    .await
    .map_err(OidcError::Database)?;
    let mut identity = AuthenticatedIdentity::new(
        UserId::from_uuid(user_id),
        verified.issuer.clone(),
        verified.subject.clone(),
        verified.claims.clone(),
        request_id,
    );
    identity.trace_id = trace_id.map(str::to_owned);
    Ok(identity)
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

/// OIDC verification or identity-mapping failure.
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
    /// Verified external identity has no internal mapping.
    #[error("OIDC identity is not mapped to an internal user")]
    UnmappedIdentity,
    /// Mapped user cannot authenticate.
    #[error("mapped user is not active")]
    InactiveUser,
    /// Validated claims could not be represented as JSON.
    #[error("OIDC claims could not be represented")]
    Claims(#[source] serde_json::Error),
    /// Identity persistence failed.
    #[error("OIDC identity mapping failed")]
    Database(#[source] sqlx::Error),
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
