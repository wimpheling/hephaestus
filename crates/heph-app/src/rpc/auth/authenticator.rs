use super::{
    BOOTSTRAP_ACTOR_KIND, BOOTSTRAP_SUBJECT, BootstrapIdentity, MediatorAssertionError,
    MediatorPrincipal,
};
use crate::rpc::auth::claims::{
    BootstrapClaims, MediatorClaims, SessionBootstrapClaims, bearer_token, mediator_validation,
};
use axum::http::HeaderMap;
use identity_domain::{BrowserSessionSid, UserId};
use jsonwebtoken::{DecodingKey, decode};
use std::str::FromStr;
use uuid::Uuid;

/// Verifies audience-bound, short-lived Phoenix mediator assertions.
#[derive(Clone)]
pub struct MediatorAuthenticator {
    decoding_key: DecodingKey,
}

impl MediatorAuthenticator {
    /// Creates an authenticator from the domain-separated HS256 key.
    #[must_use]
    pub fn new(signing_key: &[u8]) -> Self {
        Self {
            decoding_key: DecodingKey::from_secret(signing_key),
        }
    }

    /// Authenticates the bearer assertion for one exact RPC procedure.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error for missing, malformed, expired,
    /// overlong, or wrong-audience assertions.
    pub fn authenticate(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
    ) -> Result<MediatorPrincipal, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let validation = mediator_validation(expected_audience);
        let claims = decode::<MediatorClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.validate_times()?;
        Ok(MediatorPrincipal {
            user_id: UserId::from_str(&claims.sub).map_err(|_| MediatorAssertionError)?,
            assertion_id: Uuid::parse_str(&claims.jti).map_err(|_| MediatorAssertionError)?,
            sid: BrowserSessionSid::from_str(claims.sid.as_deref().ok_or(MediatorAssertionError)?)
                .map_err(|_| MediatorAssertionError)?,
        })
    }

    /// Authenticates one bootstrap assertion by its verified OIDC binding.
    ///
    /// This deliberately checks only issuer and subject. The full `ResolveIdentity`
    /// bootstrap retains its display and email binding in `authenticate_bootstrap`.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error unless the signed bootstrap assertion
    /// has the expected audience, actor kind, issuer, and subject.
    pub fn authenticate_session_bootstrap(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
        issuer: &str,
        subject: &str,
    ) -> Result<Uuid, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let mut validation = mediator_validation(expected_audience);
        validation.sub = Some(String::from(BOOTSTRAP_SUBJECT));
        let claims = decode::<SessionBootstrapClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.registered.validate_times()?;
        if claims.actor_kind != BOOTSTRAP_ACTOR_KIND
            || claims.oidc_iss != issuer
            || claims.oidc_sub != subject
        {
            return Err(MediatorAssertionError);
        }
        Uuid::parse_str(&claims.registered.jti).map_err(|_| MediatorAssertionError)
    }

    /// Authenticates the method-specific identity-resolution bootstrap.
    ///
    /// # Errors
    ///
    /// Returns one non-sensitive error unless every signed identity field
    /// exactly matches the request supplied by the web mediator.
    pub fn authenticate_bootstrap(
        &self,
        headers: &HeaderMap,
        expected_audience: &str,
        expected: &BootstrapIdentity<'_>,
    ) -> Result<Uuid, MediatorAssertionError> {
        let token = bearer_token(headers)?;
        let mut validation = mediator_validation(expected_audience);
        validation.sub = Some(String::from(BOOTSTRAP_SUBJECT));
        let claims = decode::<BootstrapClaims>(token, &self.decoding_key, &validation)
            .map_err(|_| MediatorAssertionError)?
            .claims;
        claims.registered.validate_times()?;
        if claims.actor_kind != BOOTSTRAP_ACTOR_KIND
            || claims.oidc_iss != expected.issuer
            || claims.oidc_sub != expected.subject
            || claims.name != expected.display_name
            || claims.email != expected.email
            || claims.email_verified != expected.email_verified
        {
            return Err(MediatorAssertionError);
        }
        Uuid::parse_str(&claims.registered.jti).map_err(|_| MediatorAssertionError)
    }
}
