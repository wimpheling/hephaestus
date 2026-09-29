//! Registry authorities and content digests.
use crate::{RegistryValueError, errors::split_authority};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

/// A canonical DNS-style registry authority, optionally with a port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegistryAuthority(String);

impl RegistryAuthority {
    /// Parses a canonical registry authority.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidAuthority`] when the authority has
    /// a scheme, path, non-canonical casing, or invalid host/port syntax.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let Some((host, port)) = split_authority(&value) else {
            return Err(RegistryValueError::InvalidAuthority);
        };
        let valid_host = !host.is_empty()
            && host.len() <= 253
            && host.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
            })
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        let valid_port = port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        });
        (valid_host && valid_port)
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidAuthority)
    }

    /// Returns the canonical authority.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RegistryAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for RegistryAuthority {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RegistryAuthority> for String {
    fn from(value: RegistryAuthority) -> Self {
        value.0
    }
}

/// An immutable lowercase SHA-256 OCI digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Parses a canonical `sha256:<64 lowercase hexadecimal characters>` digest.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidDigest`] for any other algorithm,
    /// length, casing, or character set.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(RegistryValueError::InvalidDigest);
        };
        let valid = hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidDigest)
    }

    /// Returns the canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for Sha256Digest {
    type Err = RegistryValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.to_owned())
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<Sha256Digest> for String {
    fn from(value: Sha256Digest) -> Self {
        value.0
    }
}
