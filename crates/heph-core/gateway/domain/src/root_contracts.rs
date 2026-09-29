//! Provider-neutral gateway routing values and edge ports.

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, net::IpAddr, sync::Arc, time::Duration};
use uuid::Uuid;
use vm_trait::PrivateMailboxPublication;

use crate::Exposure;

/// Reserved public path prefix owned by gateway routing.
pub const GATEWAY_NAMESPACE: &str = "/gateway/";
/// Provider-controlled forwarding headers which never reach a gateway.
pub const UNTRUSTED_FORWARDING_HEADERS: [&str; 4] = [
    "forwarded",
    "x-forwarded-for",
    "x-forwarded-host",
    "x-forwarded-proto",
];

/// Stable identity of a derived Caddy configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GatewayConfigRevision(Uuid);

impl GatewayConfigRevision {
    /// Creates an opaque derived configuration revision.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Reconstitutes a control-plane-derived configuration revision.
    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }
}

impl Default for GatewayConfigRevision {
    fn default() -> Self {
        Self::new()
    }
}

/// An authoritative desired configuration revision and its complete route set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayDesiredConfiguration {
    /// Immutable desired revision supplied by the control plane.
    pub revision: GatewayConfigRevision,
    /// Every enabled gateway route.  Absence removes a previously derived route.
    pub routes: Vec<GatewayRouteBinding>,
}

/// One enabled route selected by the control plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayRouteBinding {
    /// Stable authoritative route identity; never derived from public input.
    pub route_id: Uuid,
    /// Exact immutable released handler revision to invoke.
    pub gateway_revision_id: Uuid,
    /// Declared exposure carried from the authoritative revision. Reserved
    /// authenticated routes must never enter the public Caddy or dispatcher
    /// path.
    pub exposure: Exposure,
    /// Normalized public path prefix below `/gateway/`.
    pub path_prefix: String,
    /// Permitted canonical HTTP methods.
    pub methods: BTreeSet<Method>,
    /// Per-route resource and execution limits.
    pub limits: GatewayLimits,
}

impl GatewayRouteBinding {
    /// Validates this binding is safe to place below the shared gateway namespace.
    ///
    /// # Errors
    ///
    /// Returns an error when the route name, methods, or limits are unsafe.
    pub fn validate(&self) -> Result<(), GatewayEdgeError> {
        validate_gateway_prefix(&self.path_prefix)?;
        if self.methods.is_empty() {
            return Err(GatewayEdgeError::InvalidRoute("methods cannot be empty"));
        }
        self.limits.validate()
    }

    /// Returns the public path prefix owned by Caddy.
    #[must_use]
    pub fn public_path(&self) -> String {
        format!("{GATEWAY_NAMESPACE}{}", self.path_prefix)
    }
}

/// Explicit bounds for a route and its invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayLimits {
    /// Maximum request body bytes.
    pub max_request_body_bytes: usize,
    /// Maximum response body bytes.
    pub max_response_body_bytes: usize,
    /// Maximum distinct request headers.
    pub max_request_headers: usize,
    /// Maximum distinct response headers.
    pub max_response_headers: usize,
    /// Maximum complete request path and query length.
    pub max_path_and_query_bytes: usize,
    /// Maximum startup and handler execution duration.
    pub execution_timeout: Duration,
}

impl GatewayLimits {
    /// Validates all limits fail closed rather than silently becoming unlimited.
    ///
    /// # Errors
    ///
    /// Returns an error when any route bound is zero.
    pub const fn validate(self) -> Result<(), GatewayEdgeError> {
        if self.max_request_body_bytes == 0
            || self.max_response_body_bytes == 0
            || self.max_request_headers == 0
            || self.max_response_headers == 0
            || self.max_path_and_query_bytes == 0
            || self.execution_timeout.is_zero()
        {
            return Err(GatewayEdgeError::InvalidRoute(
                "gateway limits must be nonzero",
            ));
        }
        Ok(())
    }
}

/// Trusted metadata produced only by a concrete edge adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedRequestMetadata {
    /// Scheme terminated by the public edge.
    pub scheme: GatewayScheme,
    /// Public listener authority selected by the adapter, never `Host` input.
    pub authority: String,
    /// Immediate client address observed by the trusted edge.
    pub client_address: IpAddr,
    /// Edge-generated opaque request correlation identifier.
    pub request_id: Uuid,
}

/// The public scheme observed by the trusted adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayScheme {
    /// HTTP.
    Http,
    /// HTTPS terminated at Caddy.
    Https,
}

/// Canonical normalized inbound HTTP request sent to a gateway handler.
#[derive(Debug, Clone)]
pub struct GatewayRequest {
    /// HTTP method.
    pub method: Method,
    /// Normalized path and optional query, always beginning with `/`.
    pub path_and_query: String,
    /// Allowlisted non-forwarding headers.
    pub headers: HeaderMap,
    /// Complete bounded body; streaming is unsupported.
    pub body: Bytes,
    /// Metadata trusted from the edge adapter.
    pub trusted: TrustedRequestMetadata,
}

/// Canonical bounded response returned by a handler.
#[derive(Debug, Clone)]
pub struct GatewayResponse {
    /// HTTP status selected by released application code.
    pub status: StatusCode,
    /// Allowlisted response headers.
    pub headers: HeaderMap,
    /// Complete bounded response body; streaming is unsupported.
    pub body: Bytes,
    /// At most one host-authorized mailbox publication candidate from the
    /// released handler. This is never exposed to the public HTTP client.
    pub mailbox_publication: Option<PrivateMailboxPublication>,
}

/// Safe outcome returned to the provider adapter.
#[derive(Debug, Clone)]
pub struct GatewayProviderResponse {
    /// Response safe for the public client.
    pub response: GatewayResponse,
    /// Opaque invocation correlation identity for logs only.
    pub invocation_id: Uuid,
}

/// Desired configuration reconciler for any edge provider.
#[async_trait]
pub trait GatewayProvider: Send + Sync {
    /// Applies the complete desired route set atomically, or leaves the prior
    /// observed revision unchanged.  Implementations must make duplicate
    /// revision application idempotent.
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError>;
    /// Reapplies the complete authoritative configuration after an edge
    /// restart. Implementations that can prove their observation is still
    /// present may use the ordinary idempotent reconciliation path.
    async fn recover(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        self.reconcile(desired).await
    }
    /// Translates one provider request into canonical HTTP and dispatches it.
    /// Only the adapter may populate trusted metadata.
    async fn forward(&self, request: GatewayRequest) -> GatewayProviderResponse;
}

/// Dispatcher port used by a concrete provider after it has established trusted metadata.
#[async_trait]
pub trait GatewayRequestDispatcher: Send + Sync {
    /// Dispatches one normalized, bounded private request.
    async fn dispatch(&self, request: GatewayRequest) -> GatewayProviderResponse;
}

#[async_trait]
impl<T> GatewayRequestDispatcher for Arc<T>
where
    T: GatewayRequestDispatcher + ?Sized,
{
    async fn dispatch(&self, request: GatewayRequest) -> GatewayProviderResponse {
        (**self).dispatch(request).await
    }
}

/// Resolves an exact authoritative enabled route. The adapter must not use a
/// caller-controlled route identifier or forwarding header to choose a route.
#[async_trait]
pub trait GatewayRouteResolver: Send + Sync {
    /// Resolves a public normalized route path to its enabled binding.
    async fn resolve(
        &self,
        path_and_query: &str,
    ) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError>;
}

/// Gateway reconciliation or dispatch failure. Public adapters must map this
/// to a generic safe response rather than exposing internals.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum GatewayEdgeError {
    /// Route data violates the canonical contract.
    #[error("invalid gateway route: {0}")]
    InvalidRoute(&'static str),
    /// Request or response violates a declared bound.
    #[error("gateway HTTP contract violation: {0}")]
    Contract(&'static str),
    /// Caddy administration or a durable adapter failed.
    #[error("gateway edge unavailable")]
    Unavailable,
    /// The Caddy administration endpoint is not a loopback HTTP address.
    #[error("Caddy administration endpoint must be a loopback HTTP URL")]
    InvalidAdministrationEndpoint,
    /// The operator-owned shared Caddy baseline lacks the exact gateway slot.
    #[error("invalid shared Caddy configuration template")]
    InvalidCaddyConfiguration,
    /// VM startup or private handler invocation failed.
    #[error("gateway handler unavailable")]
    HandlerUnavailable,
    /// An inbound real-secret matcher did not pass its exact host-side lease.
    #[error("gateway inbound secret was rejected")]
    SecretRejected,
}

fn validate_gateway_prefix(prefix: &str) -> Result<(), GatewayEdgeError> {
    if prefix.is_empty()
        || prefix.starts_with('/')
        || prefix.ends_with('/')
        || prefix.contains("//")
        || prefix
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
    {
        return Err(GatewayEdgeError::InvalidRoute(
            "path prefix must be normalized relative path",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> GatewayLimits {
        GatewayLimits {
            max_request_body_bytes: 1024,
            max_response_body_bytes: 1024,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: Duration::from_secs(1),
        }
    }

    fn route(path_prefix: &str, methods: impl IntoIterator<Item = Method>) -> GatewayRouteBinding {
        GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            exposure: Exposure::Public,
            path_prefix: path_prefix.into(),
            methods: methods.into_iter().collect(),
            limits: limits(),
        }
    }

    #[test]
    fn validates_route_shape_and_public_path() {
        let valid_route = route("orders", [Method::GET]);
        assert_eq!(valid_route.public_path(), "/gateway/orders");
        assert!(valid_route.validate().is_ok());

        assert!(matches!(
            route("/orders", [Method::GET]).validate(),
            Err(GatewayEdgeError::InvalidRoute(
                "path prefix must be normalized relative path"
            ))
        ));
        assert!(matches!(
            route("orders", []).validate(),
            Err(GatewayEdgeError::InvalidRoute("methods cannot be empty"))
        ));
    }

    #[test]
    fn rejects_zero_route_limits() {
        let mut valid_route = route("orders", [Method::POST]);
        valid_route.limits.max_response_body_bytes = 0;
        assert!(matches!(
            valid_route.validate(),
            Err(GatewayEdgeError::InvalidRoute(
                "gateway limits must be nonzero"
            ))
        ));
    }
}
