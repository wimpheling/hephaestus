//! Private shared-Caddy reconciliation and bounded synchronous gateway dispatch.
//!
//! This crate deliberately knows no database schema, authorization relation, or
//! Caddy admin endpoint.  Its ports let the gateway control plane supply only
//! already-authorized active routes and let the host own the private Caddy
//! administration boundary.

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, HeaderName, Method, StatusCode};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    net::IpAddr,
    sync::Arc,
    time::Duration,
};
use tokio::time::timeout;
use uuid::Uuid;
use vm_trait::{PrivateHttpRequest, PrivateHttpResponse, VmInstance};

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
    /// Translates one provider request into canonical HTTP and dispatches it.
    /// Only the adapter may populate trusted metadata.
    async fn forward(&self, request: GatewayRequest) -> GatewayProviderResponse;
}

/// Private Caddy administration port.  Implementations must not be exposed to
/// gateway VMs or public request paths.
#[async_trait]
pub trait CaddyAdministration: Send + Sync {
    /// Atomically loads a complete derived Caddy configuration.
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError>;
}

#[async_trait]
impl<T> CaddyAdministration for Arc<T>
where
    T: CaddyAdministration + ?Sized,
{
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError> {
        (**self).load(configuration).await
    }
}

/// Minimal Caddy adapter: it derives `/gateway/` routes only and forwards
/// private normalized HTTP to a stable dispatcher.
pub struct LocalCaddyGatewayProvider<A, D> {
    administration: A,
    dispatcher: D,
    observed: tokio::sync::RwLock<Option<GatewayConfigRevision>>,
}

impl<A, D> LocalCaddyGatewayProvider<A, D> {
    /// Constructs an adapter with no observed configuration after a restart.
    #[must_use]
    pub const fn new(administration: A, dispatcher: D) -> Self {
        Self {
            administration,
            dispatcher,
            observed: tokio::sync::RwLock::const_new(None),
        }
    }

    /// Returns the last successfully applied revision.  `None` forces safe
    /// deterministic reconciliation after a process restart.
    pub async fn observed_revision(&self) -> Option<GatewayConfigRevision>
    where
        A: Sync,
        D: Sync,
    {
        *self.observed.read().await
    }
}

#[async_trait]
impl<A, D> GatewayProvider for LocalCaddyGatewayProvider<A, D>
where
    A: CaddyAdministration,
    D: GatewayRequestDispatcher,
{
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        for route in &desired.routes {
            route.validate()?;
        }
        ensure_unique_routes(&desired.routes)?;
        if self.observed_revision().await == Some(desired.revision) {
            return Ok(desired.revision);
        }
        let config = caddy_configuration(desired)?;
        self.administration.load(config).await?;
        *self.observed.write().await = Some(desired.revision);
        Ok(desired.revision)
    }

    async fn forward(&self, request: GatewayRequest) -> GatewayProviderResponse {
        self.dispatcher.dispatch(request).await
    }
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

/// Resolves an exact authoritative enabled route.  The adapter must not use a
/// caller-controlled route identifier or forwarding header to choose a route.
#[async_trait]
pub trait GatewayRouteResolver: Send + Sync {
    /// Resolves a public normalized route path to its enabled binding.
    async fn resolve(
        &self,
        path_and_query: &str,
    ) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError>;
}

/// Runs the exact released handler revision in a short-lived isolated VM.
#[async_trait]
pub trait GatewayVmHandler: Send + Sync {
    /// Invokes private HTTP for the exact immutable binding.
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError>;
}

/// Starts an exact released gateway VM.  Gateway control-plane code supplies
/// the immutable release mount and capability bootstrap; this edge crate never
/// selects a release from public request data.
#[async_trait]
pub trait GatewayVmLauncher: Send + Sync {
    /// Starts and returns an isolated VM for the exact immutable route binding.
    async fn launch(
        &self,
        route: &GatewayRouteBinding,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError>;
}

/// Bridges canonical gateway HTTP to a VM provider's private host-to-guest
/// handler transport.  It does not create a guest listener or port forward.
pub struct PrivateHttpVmGatewayHandler<L> {
    launcher: L,
}

impl<L> PrivateHttpVmGatewayHandler<L> {
    /// Creates an adapter from the release-aware VM launcher.
    #[must_use]
    pub const fn new(launcher: L) -> Self {
        Self { launcher }
    }
}

#[async_trait]
impl<L> GatewayVmHandler for PrivateHttpVmGatewayHandler<L>
where
    L: GatewayVmLauncher,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let instance = self.launcher.launch(route).await?;
        let invocation = async {
            instance
                .start()
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
            let response = instance
                .invoke_private_http(PrivateHttpRequest {
                    method: request.method,
                    path_and_query: request.path_and_query,
                    headers: request.headers,
                    body: request.body,
                })
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
            Ok::<_, GatewayEdgeError>(gateway_response(response))
        }
        .await;
        let cleanup = instance.destroy().await;
        if cleanup.is_err() && invocation.is_ok() {
            return Err(GatewayEdgeError::HandlerUnavailable);
        }
        invocation
    }
}

/// Durable audit/session boundary for one invocation.  The control plane owns
/// its authority snapshot and never accepts these values from the public edge.
#[async_trait]
pub trait GatewayInvocationRecorder: Send + Sync {
    /// Records a pre-launch invocation/session and returns its opaque id.
    async fn accepted(
        &self,
        route: &GatewayRouteBinding,
        request_id: Uuid,
    ) -> Result<Uuid, GatewayEdgeError>;
    /// Records the terminal safe outcome.
    async fn completed(
        &self,
        invocation_id: Uuid,
        outcome: GatewayInvocationOutcome,
    ) -> Result<(), GatewayEdgeError>;
}

/// Persistable terminal outcome with no request/response payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayInvocationOutcome {
    /// Handler returned a bounded response.
    Completed,
    /// Handler/startup failed.
    Failed,
    /// Deadline elapsed.
    TimedOut,
    /// Request was rejected before launch.
    Rejected,
}

/// A synchronous dispatcher with deliberate response fallbacks.
pub struct GatewayDispatcher<R, H, I> {
    resolver: R,
    handler: H,
    recorder: I,
}

impl<R, H, I> GatewayDispatcher<R, H, I> {
    /// Creates a dispatcher from explicit authority, VM, and audit ports.
    #[must_use]
    pub const fn new(resolver: R, handler: H, recorder: I) -> Self {
        Self {
            resolver,
            handler,
            recorder,
        }
    }
}

#[async_trait]
impl<R, H, I> GatewayRequestDispatcher for GatewayDispatcher<R, H, I>
where
    R: GatewayRouteResolver,
    H: GatewayVmHandler,
    I: GatewayInvocationRecorder,
{
    async fn dispatch(&self, request: GatewayRequest) -> GatewayProviderResponse {
        let fallback = |status| GatewayProviderResponse {
            response: empty_response(status),
            invocation_id: Uuid::nil(),
        };
        let Some(route) = (match self.resolver.resolve(&request.path_and_query).await {
            Ok(route) => route,
            Err(_) => return fallback(StatusCode::SERVICE_UNAVAILABLE),
        }) else {
            return fallback(StatusCode::NOT_FOUND);
        };
        if let Err(error) = validate_request(&route, &request) {
            let _ = error;
            return fallback(StatusCode::BAD_REQUEST);
        }
        let Ok(invocation_id) = self
            .recorder
            .accepted(&route, request.trusted.request_id)
            .await
        else {
            return fallback(StatusCode::SERVICE_UNAVAILABLE);
        };
        let result = timeout(
            route.limits.execution_timeout,
            self.handler.invoke(&route, request),
        )
        .await;
        let (response, outcome) = match result {
            Ok(Ok(response)) if validate_response(&route, &response).is_ok() => {
                (response, GatewayInvocationOutcome::Completed)
            }
            Ok(Ok(_) | Err(_)) => (
                empty_response(StatusCode::BAD_GATEWAY),
                GatewayInvocationOutcome::Failed,
            ),
            Err(_) => (
                empty_response(StatusCode::GATEWAY_TIMEOUT),
                GatewayInvocationOutcome::TimedOut,
            ),
        };
        let _ = self.recorder.completed(invocation_id, outcome).await;
        GatewayProviderResponse {
            response,
            invocation_id,
        }
    }
}

/// Gateway reconciliation or dispatch failure.  Public adapters must map this
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
    /// VM startup or private handler invocation failed.
    #[error("gateway handler unavailable")]
    HandlerUnavailable,
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
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'/'))
    {
        return Err(GatewayEdgeError::InvalidRoute(
            "path prefix must be normalized relative path",
        ));
    }
    Ok(())
}

fn ensure_unique_routes(routes: &[GatewayRouteBinding]) -> Result<(), GatewayEdgeError> {
    let mut prefixes = BTreeSet::new();
    for route in routes {
        if !prefixes.insert(route.path_prefix.as_str()) {
            return Err(GatewayEdgeError::InvalidRoute(
                "duplicate active gateway path",
            ));
        }
    }
    Ok(())
}

fn caddy_configuration(desired: &GatewayDesiredConfiguration) -> Result<Vec<u8>, GatewayEdgeError> {
    let mut routes = desired.routes.clone();
    routes.sort_by(|left, right| left.path_prefix.cmp(&right.path_prefix));
    let routes: Vec<BTreeMap<&str, String>> = routes
        .into_iter()
        .map(|route| {
            BTreeMap::from([
                ("match", route.public_path()),
                ("route_id", route.route_id.to_string()),
                ("upstream", "gateway-dispatcher.private".to_owned()),
            ])
        })
        .collect();
    serde_json::to_vec(&routes).map_err(|_| GatewayEdgeError::Unavailable)
}

fn validate_request(
    route: &GatewayRouteBinding,
    request: &GatewayRequest,
) -> Result<(), GatewayEdgeError> {
    if !request.path_and_query.starts_with('/')
        || request.path_and_query.contains("//")
        || request
            .path_and_query
            .split('?')
            .next()
            .is_some_and(|path| path.split('/').any(|part| part == "." || part == ".."))
    {
        return Err(GatewayEdgeError::Contract("ambiguous request path"));
    }
    if !request_targets_route(route, &request.path_and_query) {
        return Err(GatewayEdgeError::Contract("route mismatch"));
    }
    if !route.methods.contains(&request.method) {
        return Err(GatewayEdgeError::Contract("method is not allowed"));
    }
    if request.path_and_query.len() > route.limits.max_path_and_query_bytes
        || request.body.len() > route.limits.max_request_body_bytes
        || request.headers.len() > route.limits.max_request_headers
    {
        return Err(GatewayEdgeError::Contract("request exceeds route limits"));
    }
    if request.headers.keys().any(|name| {
        is_hop_by_hop(name)
            || UNTRUSTED_FORWARDING_HEADERS
                .iter()
                .any(|blocked| name.as_str().eq_ignore_ascii_case(blocked))
    }) {
        return Err(GatewayEdgeError::Contract("forbidden request header"));
    }
    Ok(())
}

fn request_targets_route(route: &GatewayRouteBinding, path_and_query: &str) -> bool {
    let path = path_and_query
        .split_once('?')
        .map_or(path_and_query, |(path, _)| path);
    let public_path = route.public_path();
    path == public_path
        || path
            .strip_prefix(&public_path)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn validate_response(
    route: &GatewayRouteBinding,
    response: &GatewayResponse,
) -> Result<(), GatewayEdgeError> {
    if response.body.len() > route.limits.max_response_body_bytes
        || response.headers.len() > route.limits.max_response_headers
        || response.headers.keys().any(is_hop_by_hop)
    {
        return Err(GatewayEdgeError::Contract("response violates route limits"));
    }
    Ok(())
}

fn is_hop_by_hop(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn empty_response(status: StatusCode) -> GatewayResponse {
    GatewayResponse {
        status,
        headers: HeaderMap::new(),
        body: Bytes::new(),
    }
}

fn gateway_response(response: PrivateHttpResponse) -> GatewayResponse {
    GatewayResponse {
        status: response.status,
        headers: response.headers,
        body: response.body,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http::HeaderValue;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use vm_fake::{FakeProvider, PrivateHttpResponder};
    use vm_trait::{
        GuestCommand, NetworkMode, RootFilesystem, VmError, VmProvider, VmResources, VmSpec,
    };

    fn limits() -> GatewayLimits {
        GatewayLimits {
            max_request_body_bytes: 8,
            max_response_body_bytes: 8,
            max_request_headers: 4,
            max_response_headers: 4,
            max_path_and_query_bytes: 64,
            execution_timeout: Duration::from_millis(50),
        }
    }
    fn route() -> GatewayRouteBinding {
        GatewayRouteBinding {
            route_id: Uuid::new_v4(),
            gateway_revision_id: Uuid::new_v4(),
            path_prefix: "echo".to_owned(),
            methods: BTreeSet::from([Method::POST]),
            limits: limits(),
        }
    }
    fn request(path: &str) -> GatewayRequest {
        GatewayRequest {
            method: Method::POST,
            path_and_query: path.to_owned(),
            headers: HeaderMap::new(),
            body: Bytes::from_static(b"ok"),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: "heph.test".to_owned(),
                client_address: "127.0.0.1".parse().expect("address"),
                request_id: Uuid::new_v4(),
            },
        }
    }
    struct Admin(Mutex<Vec<Vec<u8>>>);
    #[async_trait]
    impl CaddyAdministration for Admin {
        async fn load(&self, config: Vec<u8>) -> Result<(), GatewayEdgeError> {
            self.0.lock().expect("lock").push(config);
            Ok(())
        }
    }

    struct FailingAdmin;
    #[async_trait]
    impl CaddyAdministration for FailingAdmin {
        async fn load(&self, _: Vec<u8>) -> Result<(), GatewayEdgeError> {
            Err(GatewayEdgeError::Unavailable)
        }
    }
    struct Resolver(GatewayRouteBinding);
    #[async_trait]
    impl GatewayRouteResolver for Resolver {
        async fn resolve(&self, _: &str) -> Result<Option<GatewayRouteBinding>, GatewayEdgeError> {
            Ok(Some(self.0.clone()))
        }
    }
    struct Handler {
        calls: AtomicUsize,
        delay: Duration,
    }
    #[async_trait]
    impl GatewayVmHandler for Handler {
        async fn invoke(
            &self,
            _: &GatewayRouteBinding,
            _: GatewayRequest,
        ) -> Result<GatewayResponse, GatewayEdgeError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            Ok(empty_response(StatusCode::CREATED))
        }
    }
    struct Recorder;
    #[async_trait]
    impl GatewayInvocationRecorder for Recorder {
        async fn accepted(
            &self,
            _: &GatewayRouteBinding,
            _: Uuid,
        ) -> Result<Uuid, GatewayEdgeError> {
            Ok(Uuid::new_v4())
        }
        async fn completed(
            &self,
            _: Uuid,
            _: GatewayInvocationOutcome,
        ) -> Result<(), GatewayEdgeError> {
            Ok(())
        }
    }

    struct EchoHandler;
    #[async_trait]
    impl GatewayVmHandler for EchoHandler {
        async fn invoke(
            &self,
            _: &GatewayRouteBinding,
            request: GatewayRequest,
        ) -> Result<GatewayResponse, GatewayEdgeError> {
            let mut headers = HeaderMap::new();
            headers.insert("content-type", HeaderValue::from_static("text/plain"));
            headers.insert("x-gateway-request-id", HeaderValue::from_static("trusted"));
            Ok(GatewayResponse {
                status: StatusCode::ACCEPTED,
                headers,
                body: request.body,
            })
        }
    }

    struct PrivateEcho;
    #[async_trait]
    impl PrivateHttpResponder for PrivateEcho {
        async fn invoke(
            &self,
            request: PrivateHttpRequest,
        ) -> Result<PrivateHttpResponse, VmError> {
            Ok(PrivateHttpResponse {
                status: StatusCode::CREATED,
                headers: HeaderMap::new(),
                body: request.body,
            })
        }
    }

    struct FakeGatewayLauncher {
        provider: FakeProvider,
    }
    #[async_trait]
    impl GatewayVmLauncher for FakeGatewayLauncher {
        async fn launch(
            &self,
            route: &GatewayRouteBinding,
        ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError> {
            self.provider
                .provision(VmSpec {
                    id: vm_trait::VmId(format!("gateway-{}", route.route_id)),
                    root: RootFilesystem::Directory {
                        host_path: "/gateway/release-root".into(),
                    },
                    disks: Vec::new(),
                    mounts: Vec::new(),
                    resources: VmResources {
                        vcpus: 1,
                        memory_mib: 64,
                    },
                    network: NetworkMode::Disabled,
                    command: GuestCommand {
                        program: "/gateway/handler".to_owned(),
                        args: Vec::new(),
                        env: BTreeMap::new(),
                        working_dir: None,
                    },
                    runtime_authority: None,
                    labels: BTreeMap::new(),
                })
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)
        }
    }
    #[tokio::test]
    async fn reconciliation_is_deterministic_and_idempotent() {
        let admin = Arc::new(Admin(Mutex::new(Vec::new())));
        let provider = LocalCaddyGatewayProvider::new(
            Arc::clone(&admin),
            Arc::new(GatewayDispatcher::new(
                Resolver(route()),
                Handler {
                    calls: AtomicUsize::new(0),
                    delay: Duration::ZERO,
                },
                Recorder,
            )),
        );
        let desired = GatewayDesiredConfiguration {
            revision: GatewayConfigRevision::new(),
            routes: vec![route()],
        };
        provider
            .reconcile(&desired)
            .await
            .expect("first reconciliation");
        provider
            .reconcile(&desired)
            .await
            .expect("duplicate reconciliation");
        assert_eq!(admin.0.lock().expect("lock").len(), 1);
    }

    #[tokio::test]
    async fn failed_reconciliation_does_not_advance_observed_revision() {
        let provider = LocalCaddyGatewayProvider::new(
            FailingAdmin,
            GatewayDispatcher::new(
                Resolver(route()),
                Handler {
                    calls: AtomicUsize::new(0),
                    delay: Duration::ZERO,
                },
                Recorder,
            ),
        );
        let desired = GatewayDesiredConfiguration {
            revision: GatewayConfigRevision::new(),
            routes: vec![route()],
        };
        assert!(provider.reconcile(&desired).await.is_err());
        assert_eq!(provider.observed_revision().await, None);
    }
    #[tokio::test]
    async fn dispatcher_times_out_and_never_relays_unbounded_handler() {
        let dispatcher = GatewayDispatcher::new(
            Resolver(route()),
            Handler {
                calls: AtomicUsize::new(0),
                delay: Duration::from_secs(1),
            },
            Recorder,
        );
        let response = dispatcher.dispatch(request("/gateway/echo")).await;
        assert_eq!(response.response.status, StatusCode::GATEWAY_TIMEOUT);
    }
    #[tokio::test]
    async fn dispatcher_rejects_forwarded_header_before_launch() {
        let handler = Handler {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        };
        let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder);
        let mut inbound = request("/gateway/echo");
        inbound
            .headers
            .insert("x-forwarded-for", HeaderValue::from_static("attacker"));
        assert_eq!(
            dispatcher.dispatch(inbound).await.response.status,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn shared_caddy_seam_relays_bounded_status_headers_and_body() {
        let provider = LocalCaddyGatewayProvider::new(
            Admin(Mutex::new(Vec::new())),
            GatewayDispatcher::new(Resolver(route()), EchoHandler, Recorder),
        );
        let response = provider
            .forward(request("/gateway/echo?source=caddy"))
            .await;
        assert_ne!(response.invocation_id, Uuid::nil());
        assert_eq!(response.response.status, StatusCode::ACCEPTED);
        assert_eq!(response.response.body, Bytes::from_static(b"ok"));
        assert_eq!(
            response.response.headers.get("x-gateway-request-id"),
            Some(&HeaderValue::from_static("trusted"))
        );
    }

    #[tokio::test]
    async fn dispatcher_does_not_treat_a_prefix_collision_as_its_route() {
        let handler = Handler {
            calls: AtomicUsize::new(0),
            delay: Duration::ZERO,
        };
        let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder);
        assert_eq!(
            dispatcher
                .dispatch(request("/gateway/echo-unrelated"))
                .await
                .response
                .status,
            StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn dispatcher_bridges_to_a_private_vm_http_handler_without_networking() {
        let handler = PrivateHttpVmGatewayHandler::new(FakeGatewayLauncher {
            provider: FakeProvider::new().with_private_http_responder(Arc::new(PrivateEcho)),
        });
        let dispatcher = GatewayDispatcher::new(Resolver(route()), handler, Recorder);
        let response = dispatcher.dispatch(request("/gateway/echo")).await;
        assert_eq!(response.response.status, StatusCode::CREATED);
        assert_eq!(response.response.body, Bytes::from_static(b"ok"));
    }
}
