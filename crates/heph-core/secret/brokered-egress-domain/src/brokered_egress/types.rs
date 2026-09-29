use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

use super::{BrokeredEgressError, MAX_HEADER_NAME_BYTES};

macro_rules! identifier {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            /// Reconstitutes an identifier from a UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    BrokeredSecretRuleId,
    "An immutable placeholder-substitution rule."
);
identifier!(PlaceholderId, "A non-secret stable placeholder identity.");

/// Exact normalized HTTPS origin; paths, wildcards, credentials, and IP
/// literals are forbidden so TLS identity has one unambiguous hostname.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ExactHttpsOrigin(String);

impl ExactHttpsOrigin {
    /// Parses and normalizes one exact HTTPS origin.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::InvalidOrigin`] for a non-HTTPS URL,
    /// wildcard, IP literal, path, query, fragment, or invalid DNS hostname.
    pub fn parse(value: impl Into<String>) -> Result<Self, BrokeredEgressError> {
        let value = value.into();
        let Some(authority) = value.strip_prefix("https://") else {
            return Err(BrokeredEgressError::InvalidOrigin);
        };
        if authority.is_empty()
            || authority.contains(['/', '?', '#', '@'])
            || authority.contains(char::is_whitespace)
        {
            return Err(BrokeredEgressError::InvalidOrigin);
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) if !host.contains(':') => (host, Some(port)),
            Some(_) => return Err(BrokeredEgressError::InvalidOrigin),
            None => (authority, None),
        };
        let host = host.to_ascii_lowercase();
        if !valid_hostname(&host) {
            return Err(BrokeredEgressError::InvalidOrigin);
        }
        let canonical = match port {
            None | Some("443") => format!("https://{host}"),
            Some(port) if valid_port(port) => format!("https://{host}:{port}"),
            Some(_) => return Err(BrokeredEgressError::InvalidOrigin),
        };
        Ok(Self(canonical))
    }

    /// Returns the canonical HTTPS origin.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for ExactHttpsOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl TryFrom<String> for ExactHttpsOrigin {
    type Error = BrokeredEgressError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<ExactHttpsOrigin> for String {
    fn from(value: ExactHttpsOrigin) -> Self {
        value.0
    }
}

/// A case-normalized HTTP field-name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct HeaderName(String);
impl HeaderName {
    /// Parses one RFC-token-like header name and normalizes it to lowercase.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::InvalidHeader`] for an empty, oversized,
    /// or non-token field name.
    pub fn parse(value: impl Into<String>) -> Result<Self, BrokeredEgressError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_HEADER_NAME_BYTES || !value.bytes().all(is_token) {
            return Err(BrokeredEgressError::InvalidHeader);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
    /// Returns the normalized lowercase name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl fmt::Display for HeaderName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
impl TryFrom<String> for HeaderName {
    type Error = BrokeredEgressError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<HeaderName> for String {
    fn from(value: HeaderName) -> Self {
        value.0
    }
}

fn valid_port(value: &str) -> bool {
    value.parse::<u16>().is_ok_and(|port| port != 0)
        && value.bytes().all(|byte| byte.is_ascii_digit())
}
fn valid_hostname(value: &str) -> bool {
    (1..=253).contains(&value.len())
        && !value.ends_with('.')
        && value.parse::<std::net::IpAddr>().is_err()
        && value.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}
const fn is_token(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
        || matches!(
            byte,
            b'!' | b'#'
                | b'$'
                | b'%'
                | b'&'
                | b'\''
                | b'*'
                | b'+'
                | b'-'
                | b'.'
                | b'^'
                | b'_'
                | b'`'
                | b'|'
                | b'~'
        )
}
