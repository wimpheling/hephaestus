//! Authenticated UI-origin content serving.
//!
//! This boundary accepts a raw HTTP path and method. It never accepts a
//! caller-selected declaration kind: the app-pool adapter classifies the
//! exact published generation and invokes the existing typed verifier for each
//! candidate, requiring exactly one eligible declaration. Query text is kept
//! opaque and is excluded from declaration matching.
//!
//! Intended home: a later `hephaestus-app` UI-origin handler module after the
//! host/read-port and gateway admission seams are integrated.

use async_trait::async_trait;
use axum::{
    Router,
    body::{Body, Bytes},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
};
use gateway_domain::HttpMethod;
use identity_domain::RequestId;
use release_artifact_store::LocalArtifactStore;
use release_domain::UiInstallationGenerationId;
use release_service::ui_browser_host::{UiNamespace, UiPublicPort};
use release_service::ui_browser_serving::{
    UiBrowserHttpServingProjection, UiGatewayRequestProjection, UiGenerationHostResolver,
};
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

use crate::{ui_audit::UiAuditRecorder, ui_origin_config::UiOriginConfig};

/// Bound for a guest request body forwarded to the managed/API worker.
pub const MAX_UI_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Bound for a managed/API response returned by the later gateway seam.
pub const MAX_UI_RESPONSE_BODY_BYTES: usize = 2 * 1024 * 1024;
/// Bound for an opaque query suffix retained for a managed/API request.
pub const MAX_UI_QUERY_BYTES: usize = 8 * 1024;
/// Default cap for one verified static object served by this boundary.
pub const DEFAULT_MAX_STATIC_BYTES: u64 = 32 * 1024 * 1024;
/// Blocking file reads are isolated from the async runtime.
pub const DEFAULT_STATIC_READ_CONCURRENCY: usize = 16;
/// One content request has one bounded authority/read/dispatch deadline.
pub const DEFAULT_CONTENT_DEADLINE: Duration = Duration::from_secs(30);

/// Canonical request input after transport parsing. `path` excludes the
/// opaque query; the adapter must classify only this path and method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiHttpRequest {
    /// Canonical method vocabulary used by release declarations.
    pub method: HttpMethod,
    /// Canonical absolute path with no query, encoding, or traversal.
    pub path: release_service::UiBrowserHttpPath,
    /// Opaque query suffix, including no leading `?`.
    pub query: Option<String>,
}

impl UiHttpRequest {
    /// Parses one origin-form request without normalizing or matching query
    /// bytes. Absolute-form URI authorities and duplicate Host headers fail.
    pub fn parse(request: &Request<Body>) -> Result<Self, UiContentError> {
        if request.uri().scheme().is_some() || request.uri().authority().is_some() {
            return Err(UiContentError::InvalidRequest);
        }
        if request.headers().get_all(header::HOST).iter().count() != 1 {
            return Err(UiContentError::InvalidRequest);
        }
        let path = release_service::UiBrowserHttpPath::parse(request.uri().path().to_owned())
            .map_err(|_| UiContentError::InvalidRequest)?;
        if path.as_str() == "/_heph" || path.as_str().starts_with("/_heph/") {
            return Err(UiContentError::NotFound);
        }
        let query = request.uri().query().map(str::to_owned);
        if query
            .as_deref()
            .is_some_and(|query| query.len() > MAX_UI_QUERY_BYTES)
        {
            return Err(UiContentError::BodyTooLarge);
        }
        Ok(Self {
            method: map_method(request.method()).ok_or(UiContentError::InvalidRequest)?,
            path,
            query,
        })
    }

    /// Returns the path and opaque query in gateway origin form.
    #[must_use]
    pub fn path_and_query(&self) -> String {
        self.query.as_ref().map_or_else(
            || self.path.as_str().to_owned(),
            |query| format!("{}?{query}", self.path.as_str()),
        )
    }
}

/// Safe gateway authority handed to the later UI gateway edge. It contains no
/// raw cookie, parent session identity, digest, or caller-selected revision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UiGatewayDispatchAuthority {
    /// Correlates verifier and gateway invocation provenance.
    pub request_id: RequestId,
    /// Safe child-session identity retained for gateway audit/admission.
    pub child_session_id: release_domain::ui_browser::UiBrowserSessionId,
    /// Current actor proven by the app-pool verifier.
    pub actor_id: identity_domain::UserId,
    /// Current installation owner organization.
    pub organization_id: identity_domain::OrganizationId,
    /// Current UI installation.
    pub installation_id: release_domain::UiInstallationId,
    /// Exact current generation.
    pub generation_id: UiInstallationGenerationId,
    /// Typed declaration selected by the adapter.
    pub request: UiGatewayRequestProjection,
}

/// Safe response returned by the future UI gateway adapter. The adapter must
/// recheck binding/gateway generation authority in the same invocation path.
#[derive(Debug, Clone)]
pub struct UiGatewayResponse {
    /// Released handler status.
    pub status: StatusCode,
    /// Response headers after guest policy validation.
    pub headers: HeaderMap,
    /// Bounded response body.
    pub body: Bytes,
}

/// Safe response plus authoritative gateway disposition. It deliberately
/// carries no duplicate provider response, guest payload, gateway ID, or
/// revision ID.
#[derive(Clone)]
pub struct UiGatewayDispatchResult {
    /// Response after guest policy validation.
    pub response: UiGatewayResponse,
    /// Admission/execution classification from gateway-edge.
    pub disposition: gateway_edge::UiDispatchDisposition,
}

/// Minimal gateway dispatch seam; no runtime implementation is claimed here.
#[async_trait]
pub trait UiGatewayDispatcher: Send + Sync {
    /// Dispatches only a safe authority and sanitized request envelope while
    /// retaining the authoritative gateway disposition for audit.
    async fn dispatch_ui_detailed(
        &self,
        authority: UiGatewayDispatchAuthority,
        method: HttpMethod,
        path_and_query: String,
        headers: HeaderMap,
        body: Bytes,
    ) -> Result<UiGatewayDispatchResult, UiContentError>;
}

/// Handler dependencies and bounded resource limits.
pub struct UiContentState {
    /// Metadata-free exact-host preauth resolver.
    pub host_resolver: Arc<dyn UiGenerationHostResolver>,
    /// App-pool child verifier and declaration classifier.
    pub authority: Arc<dyn UiBrowserHttpServingProjection>,
    /// Verified local immutable artifact store.
    pub artifacts: Arc<LocalArtifactStore>,
    /// Authenticated gateway dispatch boundary.
    pub gateway: Arc<dyn UiGatewayDispatcher>,
    /// Canonical host namespace.
    pub namespace: UiNamespace,
    /// Configured public HTTPS port.
    pub public_port: UiPublicPort,
    platform_csp: HeaderValue,
    /// Maximum published static bytes accepted by this listener.
    pub max_static_bytes: u64,
    /// Deadline for one authority/read/dispatch operation.
    pub deadline: Duration,
    /// Blocking artifact read bound.
    pub static_read_permits: Arc<Semaphore>,
    /// Closed audit sink; production composition installs the worker-backed repository.
    audit: UiAuditRecorder,
}

impl UiContentState {
    /// Constructs bounded serving state. A zero bound is rejected.
    // The constructor keeps the independent projection and storage seams
    // explicit so each serving dependency remains replaceable in tests.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        authority: Arc<dyn UiBrowserHttpServingProjection>,
        artifacts: Arc<LocalArtifactStore>,
        gateway: Arc<dyn UiGatewayDispatcher>,
        namespace: UiNamespace,
        public_port: UiPublicPort,
        platform_origin: impl Into<String>,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Result<Self, UiContentError> {
        if DEFAULT_STATIC_READ_CONCURRENCY == 0 || DEFAULT_MAX_STATIC_BYTES == 0 {
            return Err(UiContentError::Unavailable);
        }
        let origin = UiOriginConfig::new(namespace, public_port, platform_origin)
            .map_err(|_| UiContentError::Unavailable)?;
        let namespace = origin.namespace().clone();
        let public_port = origin.public_port();
        let platform_origin = origin.platform_origin().to_owned();
        let platform_csp = platform_csp_value(&platform_origin)?;
        Ok(Self {
            host_resolver,
            authority,
            artifacts,
            gateway,
            namespace,
            public_port,
            platform_csp,
            max_static_bytes: DEFAULT_MAX_STATIC_BYTES,
            deadline: DEFAULT_CONTENT_DEADLINE,
            static_read_permits: Arc::new(Semaphore::new(DEFAULT_STATIC_READ_CONCURRENCY)),
            audit: UiAuditRecorder::new(audit_sink),
        })
    }
}

#[path = "ui_browser_content/error.rs"]
mod error;
#[path = "ui_browser_content/gateway_policy.rs"]
mod gateway_policy;
#[path = "ui_browser_content/gateway_response.rs"]
mod gateway_response;
#[path = "ui_browser_content/handler.rs"]
mod handler;
#[path = "ui_browser_content/host.rs"]
mod host;
#[path = "ui_browser_content/static_content.rs"]
mod static_content;

pub use error::UiContentError;
use error::error_response;
use gateway_policy::{GatewayAuditDisposition, gateway_audit_disposition};
use gateway_response::{
    apply_platform_csp, gateway_response, hex_bytes, map_method, platform_csp_value,
    redirect_response, sanitize_guest_request_headers,
};
use host::{
    map_serving_error, parse_and_resolve_host, parse_child_cookie, request_authority,
    require_unsafe_origin,
};
use static_content::{StaticRequestHeaders, serve_static};

/// Registers the content route behind the exact generation namespace. The
/// bootstrap router owns `/_heph/bootstrap`; this router owns release paths.
pub fn router(state: Arc<UiContentState>) -> Router {
    handler::router(state)
}

#[cfg(test)]
mod tests;
