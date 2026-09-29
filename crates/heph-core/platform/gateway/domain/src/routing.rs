//! Canonical gateway route and HTTP declaration values.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use super::GatewayError;

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

/// A strict origin-form path used for service readiness and health probes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceProbePath(String);

impl ServiceProbePath {
    /// Parses a bounded origin-form path without ambiguous URL syntax.
    ///
    /// # Errors
    ///
    /// Returns an error for query strings, fragments, encoding, control or
    /// whitespace characters, backslashes, repeated separators, dot segments,
    /// or trailing slashes other than the root path.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(1..=512).contains(&value.len())
            || !value.starts_with('/')
            || (value.len() > 1 && value.ends_with('/'))
            || value.contains(['?', '#', '%', '*', '\\'])
            || value.contains("//")
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            || value.split('/').any(|part| part == "." || part == "..")
        {
            return Err(GatewayError::InvalidServiceProbePath);
        }
        Ok(Self(value))
    }

    /// Returns the canonical origin-form path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ServiceProbePath {
    type Error = GatewayError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ServiceProbePath> for String {
    fn from(value: ServiceProbePath) -> Self {
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
