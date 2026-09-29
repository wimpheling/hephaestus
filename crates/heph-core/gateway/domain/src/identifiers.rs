//! Stable gateway identifiers, names, and inbound secret ports.

use async_trait::async_trait;
use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};
use uuid::Uuid;

use super::GatewayError;

/// One host-only exact inbound secret substitution rule.
pub struct InboundGatewaySecretRule {
    /// Header whose exact inbound value is matched.
    pub header: HeaderName,
    /// Decrypted value held only until dispatch completes.
    pub expected: Vec<u8>,
    /// Non-secret value delivered to the guest.
    pub placeholder: HeaderValue,
}

impl InboundGatewaySecretRule {
    /// Creates a non-empty exact substitution rule.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayError::InvalidInboundSecretRule`] when either the
    /// expected value or safe replacement is empty.
    pub fn new(
        header: HeaderName,
        expected: Vec<u8>,
        placeholder: HeaderValue,
    ) -> Result<Self, GatewayError> {
        if expected.is_empty() || placeholder.as_bytes().is_empty() {
            return Err(GatewayError::InvalidInboundSecretRule);
        }
        Ok(Self {
            header,
            expected,
            placeholder,
        })
    }
}

impl Drop for InboundGatewaySecretRule {
    fn drop(&mut self) {
        self.expected.fill(0);
    }
}

/// Gateway-owned port for resolving already-authorized inbound secret rules.
#[async_trait]
pub trait GatewayInboundSecretResolver: Send + Sync {
    /// Returns rules for one accepted invocation and exact immutable route.
    async fn rules_for_invocation(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        gateway_revision_id: Uuid,
    ) -> Result<Vec<InboundGatewaySecretRule>, GatewayError>;
}

/// Maximum bytes accepted in one canonical request or response body.
pub const MAX_BODY_BYTES: u32 = 1_048_576;
/// Maximum routes declared by one gateway revision.
pub const MAX_ROUTES: usize = 32;
/// Maximum mailbox-publication slots declared by one gateway revision.
pub const MAX_MAILBOX_PUBLICATION_SLOTS: usize = 32;
/// The bounded one-request gateway HTTP handler contract.
pub const HTTP_HANDLER_CONTRACT_V1: &str = "http.v1";
/// The explicitly declared long-lived gateway HTTP service contract.
pub const HTTP_SERVICE_HANDLER_CONTRACT_V1: &str = "http.service.v1";

macro_rules! identifier {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl $name {
            /// Creates a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            /// Reconstitutes an identifier from its UUID.
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
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}
identifier!(GatewayId, "A durable project-owned gateway identity.");
identifier!(
    GatewayRevisionId,
    "An immutable gateway declaration revision."
);
identifier!(GatewayRouteId, "A durable declared gateway route.");
identifier!(
    GatewayTargetId,
    "A stable target selected by a declared route."
);
identifier!(
    GatewayReconciliationId,
    "A provider reconciliation attempt."
);

/// A stable repository-scoped gateway name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GatewayName(String);
impl GatewayName {
    /// Parses a bounded lowercase name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not a bounded lowercase identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(1..=64).contains(&value.len())
            || !value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(GatewayError::InvalidName);
        }
        Ok(Self(value))
    }
    /// Returns its normalized value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for GatewayName {
    type Error = GatewayError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(v)
    }
}
impl From<GatewayName> for String {
    fn from(value: GatewayName) -> Self {
        value.0
    }
}
