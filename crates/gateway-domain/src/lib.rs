//! Canonical, provider-neutral gateway declarations and bounded HTTP values.
//!
//! This crate deliberately contains no listener, provider, database, or VM
//! code. It is the exact shared vocabulary for the control plane and edge.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

/// Maximum bytes accepted in one canonical request or response body.
pub const MAX_BODY_BYTES: u32 = 1_048_576;
/// Maximum routes declared by one gateway revision.
pub const MAX_ROUTES: usize = 32;
/// The sole supported gateway HTTP handler contract.
pub const HTTP_HANDLER_CONTRACT_V1: &str = "http.v1";

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

/// Public exposure policy. Authenticated forwarding is deliberately reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    /// Public route.
    Public,
    /// Reserved future policy.
    HephAuthenticated,
}

/// Lifecycle state of a durable gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayLifecycle {
    /// Accepts future invocations.
    Enabled,
    /// Retained but rejects new invocations.
    Paused,
    /// Retained tombstone.
    Removed,
}

/// Bounded method accepted by the canonical contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// PATCH.
    Patch,
    /// DELETE.
    Delete,
    /// HEAD.
    Head,
    /// OPTIONS.
    Options,
}

/// A normalized, non-wildcard path prefix under the reserved gateway namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RoutePath(String);
impl RoutePath {
    /// Parses a canonical absolute path without encoding, dot segments, or trailing slash.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is ambiguous or outside the route grammar.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(2..=512).contains(&value.len())
            || !value.starts_with('/')
            || value.ends_with('/')
            || value.contains(['?', '#', '%', '*'])
            || value.split('/').any(|part| part == "." || part == "..")
        {
            return Err(GatewayError::InvalidRoutePath);
        }
        Ok(Self(value))
    }
    /// Returns the canonical path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for RoutePath {
    type Error = GatewayError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(v)
    }
}
impl From<RoutePath> for String {
    fn from(value: RoutePath) -> Self {
        value.0
    }
}

/// One bounded, non-live route request in a gateway revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteIntent {
    /// Canonical path prefix.
    pub path: RoutePath,
    /// Allowed methods.
    pub methods: BTreeSet<HttpMethod>,
}
impl RouteIntent {
    /// Validates the bounded route intent.
    ///
    /// # Errors
    ///
    /// Returns an error when no HTTP method is selected.
    pub fn new(
        path: RoutePath,
        methods: impl IntoIterator<Item = HttpMethod>,
    ) -> Result<Self, GatewayError> {
        let methods = methods.into_iter().collect::<BTreeSet<_>>();
        if methods.is_empty() {
            return Err(GatewayError::EmptyMethods);
        }
        Ok(Self { path, methods })
    }
}

/// Immutable source declaration used to install one gateway revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayDeclaration {
    /// Stable name.
    pub name: GatewayName,
    /// Exact release-agent key selected as the immutable handler target.
    pub agent_name: String,
    /// Must equal [`HTTP_HANDLER_CONTRACT_V1`].
    pub handler_contract: String,
    /// Exposure policy.
    pub exposure: Exposure,
    /// Bounded route intent.
    pub routes: Vec<RouteIntent>,
    /// Typed non-secret parameters represented as canonical JSON.
    pub parameters: serde_json::Value,
    /// Symbolic secret slot names only.
    pub secret_slots: Vec<String>,
}
impl GatewayDeclaration {
    /// Validates an installation declaration and returns its normalized identity hash.
    ///
    /// # Errors
    ///
    /// Returns an error when the declaration violates the bounded gateway contract.
    pub fn validate(&self) -> Result<[u8; 32], GatewayError> {
        if self.handler_contract != HTTP_HANDLER_CONTRACT_V1 {
            return Err(GatewayError::UnsupportedHandlerContract);
        }
        // Keep the target selector in the same repository-key grammar as
        // gateway names even when callers construct the domain value directly
        // rather than using `agent-config`.
        GatewayName::parse(self.agent_name.clone())?;
        if self.routes.is_empty()
            || self.routes.len() > MAX_ROUTES
            || self.parameters.is_null()
            || !self.parameters.is_object()
        {
            return Err(GatewayError::InvalidDeclaration);
        }
        let mut routes = BTreeSet::new();
        for route in &self.routes {
            if !routes.insert((&route.path, &route.methods)) {
                return Err(GatewayError::DuplicateRoute);
            }
        }
        if self.secret_slots.len() > 32
            || self
                .secret_slots
                .iter()
                .any(|slot| GatewayName::parse(slot.clone()).is_err())
        {
            return Err(GatewayError::InvalidSecretSlot);
        }
        let bytes = serde_json::to_vec(self).map_err(|_| GatewayError::InvalidDeclaration)?;
        Ok(Sha256::digest(bytes).into())
    }
}

/// Errors rejecting ambiguous or unsafe declaration data.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GatewayError {
    /// The gateway name is malformed.
    #[error("invalid gateway name")]
    InvalidName,
    /// The path is ambiguous or unsafe.
    #[error("invalid route path")]
    InvalidRoutePath,
    /// A route has no methods.
    #[error("route requires a method")]
    EmptyMethods,
    /// The contract is unsupported.
    #[error("unsupported gateway handler contract")]
    UnsupportedHandlerContract,
    /// The declaration is malformed.
    #[error("invalid gateway declaration")]
    InvalidDeclaration,
    /// A declaration repeats a route.
    #[error("duplicate gateway route")]
    DuplicateRoute,
    /// A secret-slot name is malformed.
    #[error("invalid gateway secret slot")]
    InvalidSecretSlot,
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_paths() {
        assert!(RoutePath::parse("/a/../b").is_err());
        assert!(RoutePath::parse("/a%2fb").is_err());
    }
    #[test]
    fn validates_a_gateway() {
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("telegram").unwrap(),
            agent_name: String::from("telegram-handler"),
            handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post])
                    .unwrap(),
            ],
            parameters: serde_json::json!({}),
            secret_slots: vec!["telegram_secret".into()],
        };
        assert!(declaration.validate().is_ok());
    }
}
