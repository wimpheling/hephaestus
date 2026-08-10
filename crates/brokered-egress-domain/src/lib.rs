//! Typed, provider-neutral contracts for HTTPS placeholder substitution.
//!
//! These values contain authority metadata only.  They intentionally cannot
//! hold secret material, arbitrary URLs, request bodies, or provider schemas.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

/// Prefix of a stable VM-visible placeholder.
pub const PLACEHOLDER_PREFIX: &str = "heph-placeholder:v1:";
/// Maximum normalized header-name length.
pub const MAX_HEADER_NAME_BYTES: usize = 64;
/// Maximum ASCII prefix bytes allowed before an outbound placeholder.
pub const MAX_HEADER_PREFIX_BYTES: usize = 256;

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

/// The only supported substitution locations. Body, query, and arbitrary
/// matching are deliberately absent from this closed vocabulary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HttpInjectionLocation {
    /// Replace a complete outbound header value equal to the placeholder.
    OutboundHeaderValue {
        /// Normalized header selected for complete-value replacement.
        header: HeaderName,
    },
    /// Replace a placeholder following one fixed outbound header prefix.
    OutboundHeaderPrefix {
        /// Normalized header selected for prefix replacement.
        header: HeaderName,
        /// Exact non-secret prefix preserved before the replacement.
        prefix: String,
    },
    /// Replace a verified inbound header before VM delivery.
    InboundGatewayHeader {
        /// Normalized inbound header selected for constant-time verification.
        header: HeaderName,
    },
}

impl HttpInjectionLocation {
    /// Validates a location's bounded prefix and returns its direction.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::InvalidHeaderPrefix`] when a prefix is
    /// too large, non-ASCII, or contains line breaks.
    pub fn normalized(self) -> Result<Self, BrokeredEgressError> {
        if let Self::OutboundHeaderPrefix { prefix, .. } = &self {
            if prefix.len() > MAX_HEADER_PREFIX_BYTES
                || !prefix.is_ascii()
                || prefix.contains(['\r', '\n'])
            {
                return Err(BrokeredEgressError::InvalidHeaderPrefix);
            }
        }
        Ok(self)
    }
    /// Returns whether this is an egress or ingress rule.
    #[must_use]
    pub const fn direction(&self) -> InjectionDirection {
        match self {
            Self::InboundGatewayHeader { .. } => InjectionDirection::Inbound,
            Self::OutboundHeaderValue { .. } | Self::OutboundHeaderPrefix { .. } => {
                InjectionDirection::Outbound
            }
        }
    }
}

/// Direction of one substitution rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InjectionDirection {
    /// VM to HTTPS upstream.
    Outbound,
    /// Public gateway to VM.
    Inbound,
}

/// One immutable, exact authority rule.
///
/// `gateway_route_id` is required only for inbound rules and forbidden for
/// outbound rules. This prevents a rule from silently applying outside its
/// declared workload/route boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrokeredSecretRule {
    /// Stable immutable rule identity.
    pub id: BrokeredSecretRuleId,
    /// Immutable secret binding UUID.
    pub binding_id: Uuid,
    /// Immutable agent revision UUID that owns the binding.
    pub instance_revision_id: Uuid,
    /// Exact immutable selected secret version UUID.
    pub secret_version_id: Uuid,
    /// Exact verified HTTPS origin for outbound rules; absent for inbound.
    pub destination: Option<ExactHttpsOrigin>,
    /// Closed bounded header location.
    pub location: HttpInjectionLocation,
    /// Exact immutable gateway route UUID for inbound rules only.
    pub gateway_route_id: Option<Uuid>,
}

impl BrokeredSecretRule {
    /// Validates cross-field authority invariants.
    ///
    /// # Errors
    ///
    /// Returns [`BrokeredEgressError::AmbiguousRuleScope`] when an outbound
    /// rule is route-bound or an inbound rule does not have an exact route.
    pub fn normalized(mut self) -> Result<Self, BrokeredEgressError> {
        self.location = self.location.normalized()?;
        match self.location.direction() {
            InjectionDirection::Outbound
                if self.destination.is_some() && self.gateway_route_id.is_none() =>
            {
                Ok(self)
            }
            InjectionDirection::Inbound
                if self.destination.is_none() && self.gateway_route_id.is_some() =>
            {
                Ok(self)
            }
            _ => Err(BrokeredEgressError::AmbiguousRuleScope),
        }
    }
    /// Derives the stable VM-visible placeholder. This is an identifier, not a
    /// bearer credential and cannot resolve any secret without this rule.
    #[must_use]
    pub fn placeholder(&self) -> String {
        format!("{PLACEHOLDER_PREFIX}{}", self.id)
    }
    /// Returns a canonical, non-secret hash for immutable snapshots/audit.
    ///
    /// # Panics
    ///
    /// Panics only if serializing these fixed, string/UUID-only domain values
    /// unexpectedly fails.
    #[must_use]
    pub fn normalized_hash(&self) -> [u8; 32] {
        let encoded = serde_json::to_vec(self).expect("brokered rule serialization is infallible");
        Sha256::digest(encoded).into()
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

/// Safe validation failure without input or secret disclosure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BrokeredEgressError {
    /// The destination is not one exact safe HTTPS origin.
    #[error("brokered destination must be an exact HTTPS origin")]
    InvalidOrigin,
    /// The header name is outside the closed HTTP token grammar.
    #[error("brokered header name is invalid")]
    InvalidHeader,
    /// The header prefix is ambiguous or unsafe.
    #[error("brokered header prefix is invalid")]
    InvalidHeaderPrefix,
    /// Destination/direction/route fields do not describe one exact scope.
    #[error("brokered rule scope is ambiguous")]
    AmbiguousRuleScope,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outbound() -> BrokeredSecretRule {
        BrokeredSecretRule {
            id: BrokeredSecretRuleId::from_uuid(Uuid::nil()),
            binding_id: Uuid::new_v4(),
            instance_revision_id: Uuid::new_v4(),
            secret_version_id: Uuid::new_v4(),
            destination: Some(
                ExactHttpsOrigin::parse("https://API.Example.test:443").expect("origin"),
            ),
            location: HttpInjectionLocation::OutboundHeaderPrefix {
                header: HeaderName::parse("Authorization").expect("header"),
                prefix: String::from("Bearer "),
            },
            gateway_route_id: None,
        }
    }

    #[test]
    fn normalizes_exact_origins_and_headers() {
        let rule = outbound().normalized().expect("rule");
        assert_eq!(
            rule.destination.as_ref().expect("destination").as_str(),
            "https://api.example.test"
        );
        assert_eq!(
            rule.placeholder(),
            "heph-placeholder:v1:00000000-0000-0000-0000-000000000000"
        );
        assert_eq!(
            HeaderName::parse("X-Signature").expect("header").as_str(),
            "x-signature"
        );
    }

    #[test]
    fn rejects_broad_or_ambiguous_rules() {
        for origin in [
            "http://api.example.test",
            "https://*.example.test",
            "https://127.0.0.1",
            "https://api.example.test/path",
            "https://[::1]",
        ] {
            assert!(ExactHttpsOrigin::parse(origin).is_err(), "{origin}");
        }
        let mut rule = outbound();
        rule.gateway_route_id = Some(Uuid::new_v4());
        assert_eq!(
            rule.normalized(),
            Err(BrokeredEgressError::AmbiguousRuleScope)
        );
        let inbound = BrokeredSecretRule {
            destination: None,
            location: HttpInjectionLocation::InboundGatewayHeader {
                header: HeaderName::parse("X-Hook-Secret").expect("header"),
            },
            gateway_route_id: Some(Uuid::new_v4()),
            ..outbound()
        };
        assert!(inbound.normalized().is_ok());
    }

    #[test]
    fn rule_hash_is_secret_free_and_stable() {
        let rule = outbound().normalized().expect("rule");
        assert_eq!(rule.normalized_hash(), rule.normalized_hash());
        let serialized = serde_json::to_string(&rule).expect("serialize");
        assert!(!serialized.contains("secret value"));
        assert!(!serialized.contains("heph-placeholder"));
    }
}
