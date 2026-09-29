use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    MAX_CLAIM_TEXT_LENGTH, RegistryAction, RegistryService, RegistryTokenError, RepositoryActions,
    RepositoryName,
};

/// A JWT issuer identifier.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TokenIssuer(String);

impl TokenIssuer {
    /// Returns the issuer identifier.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TokenIssuer {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_claim_text(value).map(|()| Self(value.to_owned()))
    }
}

impl TryFrom<String> for TokenIssuer {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<TokenIssuer> for String {
    fn from(value: TokenIssuer) -> Self {
        value.0
    }
}

/// A stable caller subject included in an issued token.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TokenSubject(String);

impl TokenSubject {
    /// Returns the stable subject text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for TokenSubject {
    type Err = RegistryTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        validate_claim_text(value).map(|()| Self(value.to_owned()))
    }
}

impl TryFrom<String> for TokenSubject {
    type Error = RegistryTokenError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<TokenSubject> for String {
    fn from(value: TokenSubject) -> Self {
        value.0
    }
}

/// Docker Distribution-compatible signed registry claims.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryTokenClaims {
    /// Exact configured token issuer.
    pub iss: TokenIssuer,
    /// Exact registry service audience.
    pub aud: RegistryService,
    /// Stable caller subject.
    pub sub: TokenSubject,
    /// Issued-at Unix timestamp.
    pub iat: u64,
    /// Not-before Unix timestamp.
    pub nbf: u64,
    /// Expiry Unix timestamp.
    pub exp: u64,
    /// Unique token identifier.
    pub jti: Uuid,
    /// Docker Distribution repository access entries.
    pub access: Vec<RegistryAccess>,
}

/// One Docker Distribution `access` claim entry.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryAccess {
    #[serde(rename = "type")]
    pub(crate) resource_type: String,
    pub(crate) name: RepositoryName,
    pub(crate) actions: Vec<RegistryAction>,
}

impl RegistryAccess {
    pub(crate) fn from_grant(repository: &RepositoryName, actions: RepositoryActions) -> Self {
        Self {
            resource_type: "repository".to_owned(),
            name: repository.clone(),
            actions: actions.actions(),
        }
    }

    /// Returns the granted repository.
    #[must_use]
    pub const fn repository(&self) -> &RepositoryName {
        &self.name
    }

    /// Returns the granted actions in canonical pull, push order.
    #[must_use]
    pub fn actions(&self) -> &[RegistryAction] {
        &self.actions
    }
}

fn validate_claim_text(value: &str) -> Result<(), RegistryTokenError> {
    if value.is_empty()
        || value.len() > MAX_CLAIM_TEXT_LENGTH
        || !value
            .bytes()
            .all(|byte| byte.is_ascii() && !byte.is_ascii_control() && byte != b' ')
    {
        Err(RegistryTokenError::InvalidClaimText)
    } else {
        Ok(())
    }
}
