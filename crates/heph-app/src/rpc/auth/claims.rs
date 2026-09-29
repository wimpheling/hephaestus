use super::{
    BOOTSTRAP_AUDIENCE, CLOCK_SKEW_SECONDS, CREATE_BOOTSTRAP_AUDIENCE, ISSUER,
    MAX_LIFETIME_SECONDS, REVOKE_AUDIENCE,
};
use http::{HeaderMap, header::AUTHORIZATION};
use jsonwebtoken::{Algorithm, Validation};
use serde::Deserialize;
use std::collections::HashSet;
use time::OffsetDateTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum MediatorAuthMode {
    Public,
    Bootstrap,
    Signed,
    Active,
}

pub(super) fn auth_mode(path: &str) -> MediatorAuthMode {
    if !path.starts_with("/hephaestus.") {
        return MediatorAuthMode::Public;
    }
    match path {
        BOOTSTRAP_AUDIENCE | CREATE_BOOTSTRAP_AUDIENCE => MediatorAuthMode::Bootstrap,
        REVOKE_AUDIENCE => MediatorAuthMode::Signed,
        _ => MediatorAuthMode::Active,
    }
}

#[cfg(test)]
pub(super) fn requires_mediator_auth(path: &str) -> bool {
    !matches!(
        auth_mode(path),
        MediatorAuthMode::Public | MediatorAuthMode::Bootstrap
    )
}

#[derive(Deserialize)]
pub(super) struct MediatorClaims {
    pub(super) sub: String,
    pub(super) jti: String,
    #[serde(default)]
    pub(super) sid: Option<String>,
    pub(super) iat: i64,
    pub(super) nbf: i64,
    pub(super) exp: i64,
}

#[derive(Deserialize)]
pub(super) struct BootstrapClaims {
    #[serde(flatten)]
    pub(super) registered: MediatorClaims,
    pub(super) actor_kind: String,
    pub(super) oidc_iss: String,
    pub(super) oidc_sub: String,
    pub(super) name: String,
    pub(super) email: String,
    pub(super) email_verified: bool,
}

#[derive(Deserialize)]
pub(super) struct SessionBootstrapClaims {
    #[serde(flatten)]
    pub(super) registered: MediatorClaims,
    pub(super) actor_kind: String,
    pub(super) oidc_iss: String,
    pub(super) oidc_sub: String,
}

impl MediatorClaims {
    pub(super) fn validate_times(&self) -> Result<(), MediatorAssertionError> {
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let lifetime = self
            .exp
            .checked_sub(self.iat)
            .ok_or(MediatorAssertionError)?;
        if !(0..=MAX_LIFETIME_SECONDS).contains(&lifetime)
            || self.nbf < self.iat
            || self.nbf > self.exp
            || self.iat > now + CLOCK_SKEW_SECONDS
            || self.exp < now - CLOCK_SKEW_SECONDS
        {
            return Err(MediatorAssertionError);
        }
        Ok(())
    }
}

/// Non-sensitive mediator authentication failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("mediator authentication failed")]
pub struct MediatorAssertionError;

pub(super) fn bearer_token(headers: &HeaderMap) -> Result<&str, MediatorAssertionError> {
    headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(MediatorAssertionError)
}

pub(super) fn mediator_validation(expected_audience: &str) -> Validation {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[ISSUER]);
    validation.set_audience(&[expected_audience]);
    validation.leeway = u64::try_from(CLOCK_SKEW_SECONDS).expect("positive clock skew");
    validation.required_spec_claims = HashSet::from([
        String::from("aud"),
        String::from("exp"),
        String::from("iat"),
        String::from("iss"),
        String::from("jti"),
        String::from("nbf"),
        String::from("sub"),
    ]);
    validation
}
