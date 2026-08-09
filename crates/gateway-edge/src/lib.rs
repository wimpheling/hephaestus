//! Private shared-Caddy reconciliation and bounded synchronous gateway dispatch.
//!
//! This crate deliberately knows no database schema or authorization relation.
//! Its ports let the gateway control plane supply only already-authorized active
//! routes while the local adapter owns the private Caddy administration boundary.

use async_trait::async_trait;
use bytes::Bytes;
use http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeSet, net::IpAddr, sync::Arc, time::Duration};
use subtle::ConstantTimeEq;
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

/// Loopback-only HTTP client for Caddy's private administration API.
///
/// This adapter deliberately exposes only whole-config `POST /load`. Gateway
/// VMs never receive its endpoint or client, and no caller can use it to issue
/// arbitrary Caddy administration requests.
#[derive(Clone)]
pub struct LocalCaddyAdministration {
    client: reqwest::Client,
    load_endpoint: reqwest::Url,
}

impl LocalCaddyAdministration {
    /// Creates an administration client for a loopback Caddy admin endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error unless `endpoint` is an HTTP URL addressed exactly to
    /// an IP loopback host. A public or DNS administration endpoint would make
    /// the shared edge control plane remotely mutable.
    pub fn new(endpoint: &str) -> Result<Self, GatewayEdgeError> {
        let mut endpoint = reqwest::Url::parse(endpoint)
            .map_err(|_| GatewayEdgeError::InvalidAdministrationEndpoint)?;
        let loopback = endpoint
            .host_str()
            .and_then(|host| host.parse::<IpAddr>().ok())
            .is_some_and(|address| address.is_loopback());
        if endpoint.scheme() != "http" || !loopback {
            return Err(GatewayEdgeError::InvalidAdministrationEndpoint);
        }
        endpoint.set_path("/load");
        endpoint.set_query(None);
        endpoint.set_fragment(None);
        Ok(Self {
            client: reqwest::Client::new(),
            load_endpoint: endpoint,
        })
    }
}

#[async_trait]
impl CaddyAdministration for LocalCaddyAdministration {
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError> {
        let response = self
            .client
            .post(self.load_endpoint.clone())
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(configuration)
            .send()
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        if response.status().is_success() {
            Ok(())
        } else {
            Err(GatewayEdgeError::Unavailable)
        }
    }
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
    dispatcher_upstream: String,
    template: Option<LocalCaddyConfigurationTemplate>,
    observed: tokio::sync::RwLock<Option<GatewayConfigRevision>>,
}

impl<A, D> LocalCaddyGatewayProvider<A, D> {
    /// Constructs an adapter with no observed configuration after a restart.
    #[must_use]
    pub fn new(administration: A, dispatcher: D) -> Self {
        Self {
            administration,
            dispatcher,
            dispatcher_upstream: String::from("gateway-dispatcher.private"),
            template: None,
            observed: tokio::sync::RwLock::const_new(None),
        }
    }

    /// Sets the private loopback upstream that the derived Caddy routes use.
    ///
    /// The caller owns binding and protecting this endpoint.  The default is
    /// retained for isolated adapter tests only; production composition uses
    /// an explicit loopback address.
    #[must_use]
    pub fn with_dispatcher_upstream(mut self, dispatcher_upstream: String) -> Self {
        self.dispatcher_upstream = dispatcher_upstream;
        self
    }

    /// Supplies the complete operator-owned shared-Caddy configuration and
    /// its one dedicated gateway subroute slot.
    #[must_use]
    pub fn with_configuration_template(
        mut self,
        template: LocalCaddyConfigurationTemplate,
    ) -> Self {
        self.template = Some(template);
        self
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
        let template = self
            .template
            .as_ref()
            .ok_or(GatewayEdgeError::Unavailable)?;
        let config = template.render(desired, &self.dispatcher_upstream)?;
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
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError>;
}

#[async_trait]
impl<T> GatewayVmHandler for Arc<T>
where
    T: GatewayVmHandler + ?Sized,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        (**self).invoke(route, invocation_id, request).await
    }
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
        invocation_id: Uuid,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError>;
}

/// Resolves one authoritative route to its exact immutable release launch
/// specification, including the gateway runtime-session bootstrap selected by
/// the control plane. Public request data is never an input to this port.
#[async_trait]
pub trait GatewayReleaseResolver: Send + Sync {
    /// Resolves the released root filesystem, command, and session bootstrap.
    async fn resolve_launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<vm_trait::VmSpec, GatewayEdgeError>;
}

/// Provisions the already-resolved private VM launch specification.
#[async_trait]
pub trait GatewayRuntimeLauncher: Send + Sync {
    /// Provisions one VM without changing its exact immutable launch inputs.
    async fn provision_gateway(
        &self,
        spec: vm_trait::VmSpec,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError>;
}

/// Minimal internal gateway runtime bridge.
///
/// It separates immutable release/session selection from VM provisioning while
/// retaining `PrivateHttpVmGatewayHandler` as the only request executor.
pub struct GatewayRuntimeService<R, L> {
    releases: R,
    launcher: L,
}

impl<R, L> GatewayRuntimeService<R, L> {
    /// Creates an injected release-resolution and VM-provisioning bridge.
    #[must_use]
    pub const fn new(releases: R, launcher: L) -> Self {
        Self { releases, launcher }
    }
}

#[async_trait]
impl<R, L> GatewayVmLauncher for GatewayRuntimeService<R, L>
where
    R: GatewayReleaseResolver,
    L: GatewayRuntimeLauncher,
{
    async fn launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError> {
        let spec = self.releases.resolve_launch(route, invocation_id).await?;
        self.launcher.provision_gateway(spec).await
    }
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
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let instance = self.launcher.launch(route, invocation_id).await?;
        // The dispatcher enforces the public execution deadline by cancelling
        // this future.  Keep the VM lifecycle in a detached task so dropping
        // that outer future cannot leak a running microVM or its provider
        // resources.  The task always destroys the one-shot instance before
        // resolving its result.
        let task = tokio::spawn(async move {
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
        });
        task.await
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
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

/// One host-only inbound header substitution ceiling for an accepted invocation.
///
/// The real value stays inside this object and is never copied into a VM request,
/// durable record, trace, diagnostic, or response.
pub struct InboundGatewaySecretRule {
    header: HeaderName,
    expected: Vec<u8>,
    placeholder: HeaderValue,
}

impl InboundGatewaySecretRule {
    /// Creates one exact header-value matcher and VM-visible placeholder.
    ///
    /// # Errors
    ///
    /// Returns a contract error for an empty secret or invalid placeholder.
    pub fn new(
        header: HeaderName,
        expected: Vec<u8>,
        placeholder: HeaderValue,
    ) -> Result<Self, GatewayEdgeError> {
        if expected.is_empty() || placeholder.as_bytes().is_empty() {
            return Err(GatewayEdgeError::Contract("invalid inbound secret rule"));
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

/// Resolves exact active inbound secret leases after an invocation has been
/// accepted. Implementations must verify route, revision, runtime session,
/// lease status, expiry, and selected secret version before returning a rule.
#[async_trait]
pub trait GatewayInboundSecretResolver: Send + Sync {
    /// Returns every active exact substitution rule for this accepted invocation.
    async fn rules_for_invocation(
        &self,
        invocation_id: Uuid,
        route: &GatewayRouteBinding,
    ) -> Result<Vec<InboundGatewaySecretRule>, GatewayEdgeError>;
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
    inbound_secrets: Option<Arc<dyn GatewayInboundSecretResolver>>,
}

impl<R, H, I> GatewayDispatcher<R, H, I> {
    /// Creates a dispatcher from explicit authority, VM, and audit ports.
    #[must_use]
    pub const fn new(resolver: R, handler: H, recorder: I) -> Self {
        Self {
            resolver,
            handler,
            recorder,
            inbound_secrets: None,
        }
    }

    /// Adds the host-only exact-lease resolver used for inbound placeholders.
    #[must_use]
    pub fn with_inbound_secret_resolver(
        mut self,
        resolver: Arc<dyn GatewayInboundSecretResolver>,
    ) -> Self {
        self.inbound_secrets = Some(resolver);
        self
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
        let Ok(request) = self
            .rewrite_inbound_secrets(invocation_id, &route, request)
            .await
        else {
            let _ = self
                .recorder
                .completed(invocation_id, GatewayInvocationOutcome::Rejected)
                .await;
            // Missing, repeated, mismatched, revoked, and expired values
            // intentionally share the same public result as an unknown route.
            return fallback(StatusCode::NOT_FOUND);
        };
        let result = timeout(
            route.limits.execution_timeout,
            self.handler.invoke(&route, invocation_id, request),
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

impl<R: Sync, H: Sync, I: Sync> GatewayDispatcher<R, H, I> {
    async fn rewrite_inbound_secrets(
        &self,
        invocation_id: Uuid,
        route: &GatewayRouteBinding,
        mut request: GatewayRequest,
    ) -> Result<GatewayRequest, GatewayEdgeError> {
        let Some(resolver) = &self.inbound_secrets else {
            return Ok(request);
        };
        for rule in resolver.rules_for_invocation(invocation_id, route).await? {
            let values = request.headers.get_all(&rule.header);
            let mut values = values.iter();
            let Some(value) = values.next() else {
                return Err(GatewayEdgeError::SecretRejected);
            };
            if values.next().is_some() || !bool::from(value.as_bytes().ct_eq(&rule.expected)) {
                return Err(GatewayEdgeError::SecretRejected);
            }
            request
                .headers
                .insert(rule.header.clone(), rule.placeholder.clone());
        }
        Ok(request)
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

/// Complete shared-Caddy configuration with one dedicated gateway subroute.
///
/// Caddy's `/load` endpoint atomically replaces its whole configuration.  To
/// avoid clobbering platform-owned routes, an operator supplies that complete
/// baseline and explicitly reserves one `group = "hephaestus.gateway"`
/// subroute. Reconciliation replaces only that subroute's nested `routes` in
/// an in-memory copy before atomically loading it.
#[derive(Clone)]
pub struct LocalCaddyConfigurationTemplate {
    base: Value,
    server: String,
}

impl LocalCaddyConfigurationTemplate {
    /// Parses and validates a complete Caddy JSON configuration.
    ///
    /// The selected server must contain exactly one route with
    /// `group: "hephaestus.gateway"` and one `subroute` handler. Platform
    /// routes remain elsewhere in the complete supplied configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the bytes are not a complete Caddy JSON
    /// configuration or do not reserve exactly one valid gateway subroute.
    pub fn new(configuration: &[u8], server: String) -> Result<Self, GatewayEdgeError> {
        if server.is_empty() {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        let base: Value = serde_json::from_slice(configuration)
            .map_err(|_| GatewayEdgeError::InvalidCaddyConfiguration)?;
        let template = Self { base, server };
        template.gateway_subroute_index()?;
        Ok(template)
    }

    fn gateway_subroute_index(&self) -> Result<usize, GatewayEdgeError> {
        let routes = self.server_routes()?;
        let indices: Vec<_> = routes
            .iter()
            .enumerate()
            .filter_map(|(index, route)| {
                (route.get("group").and_then(Value::as_str) == Some("hephaestus.gateway"))
                    .then_some(index)
            })
            .collect();
        let [index] = indices.as_slice() else {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        };
        let handler = routes[*index]
            .get("handle")
            .and_then(Value::as_array)
            .and_then(|handlers| handlers.first())
            .filter(|handler| handler.get("handler").and_then(Value::as_str) == Some("subroute"));
        if handler.is_none() {
            return Err(GatewayEdgeError::InvalidCaddyConfiguration);
        }
        Ok(*index)
    }

    fn server_routes(&self) -> Result<&Vec<Value>, GatewayEdgeError> {
        self.base
            .pointer(&format!("/apps/http/servers/{}/routes", self.server))
            .and_then(Value::as_array)
            .ok_or(GatewayEdgeError::InvalidCaddyConfiguration)
    }

    fn render(
        &self,
        desired: &GatewayDesiredConfiguration,
        dispatcher_upstream: &str,
    ) -> Result<Vec<u8>, GatewayEdgeError> {
        if dispatcher_upstream.is_empty() {
            return Err(GatewayEdgeError::Unavailable);
        }
        let mut configuration = self.base.clone();
        let slot = self.gateway_subroute_index()?;
        let routes = caddy_gateway_routes(desired, dispatcher_upstream);
        let pointer = format!(
            "/apps/http/servers/{}/routes/{slot}/handle/0/routes",
            self.server
        );
        let target = configuration
            .pointer_mut(&pointer)
            .ok_or(GatewayEdgeError::InvalidCaddyConfiguration)?;
        *target = Value::Array(routes);
        serde_json::to_vec(&configuration).map_err(|_| GatewayEdgeError::Unavailable)
    }
}

fn caddy_gateway_routes(
    desired: &GatewayDesiredConfiguration,
    dispatcher_upstream: &str,
) -> Vec<Value> {
    let mut routes = desired.routes.clone();
    routes.sort_by(|left, right| left.path_prefix.cmp(&right.path_prefix));
    routes
        .into_iter()
        .map(|route| {
            let path = route.public_path();
            serde_json::json!({
                "match": [{ "path": [path.clone(), format!("{path}/*")] }],
                "handle": [{
                    "handler": "reverse_proxy",
                    "upstreams": [{ "dial": dispatcher_upstream }]
                }],
                "terminal": true
            })
        })
        .collect()
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
    use std::{
        collections::BTreeMap,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
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

    fn caddy_template() -> LocalCaddyConfigurationTemplate {
        LocalCaddyConfigurationTemplate::new(
            serde_json::json!({
                "apps": { "http": { "servers": { "shared": {
                    "listen": ["127.0.0.1:443"],
                    "routes": [
                        { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform" }] },
                        { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                        { "handle": [{ "handler": "static_response", "status_code": 404 }] }
                    ]
                } } } }
            })
            .to_string()
            .as_bytes(),
            String::from("shared"),
        )
        .expect("valid shared Caddy template")
    }

    #[test]
    fn shared_caddy_template_requires_one_dedicated_gateway_subroute() {
        let missing = serde_json::json!({
            "apps": { "http": { "servers": { "shared": { "routes": [] } } } }
        })
        .to_string();
        assert!(matches!(
            LocalCaddyConfigurationTemplate::new(missing.as_bytes(), String::from("shared")),
            Err(GatewayEdgeError::InvalidCaddyConfiguration)
        ));
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
            _: Uuid,
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
            _: Uuid,
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

    struct InboundRules;
    #[async_trait]
    impl GatewayInboundSecretResolver for InboundRules {
        async fn rules_for_invocation(
            &self,
            _: Uuid,
            _: &GatewayRouteBinding,
        ) -> Result<Vec<InboundGatewaySecretRule>, GatewayEdgeError> {
            Ok(vec![InboundGatewaySecretRule::new(
                HeaderName::from_static("x-hook-secret"),
                b"gateway-secret-sentinel".to_vec(),
                HeaderValue::from_static("heph-placeholder:v1:gateway-proof"),
            )?])
        }
    }

    struct HeaderCapture(Mutex<Option<HeaderValue>>);
    #[async_trait]
    impl GatewayVmHandler for HeaderCapture {
        async fn invoke(
            &self,
            _: &GatewayRouteBinding,
            _: Uuid,
            request: GatewayRequest,
        ) -> Result<GatewayResponse, GatewayEdgeError> {
            *self.0.lock().expect("capture") = request.headers.get("x-hook-secret").cloned();
            Ok(empty_response(StatusCode::NO_CONTENT))
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

    struct SlowPrivateHandler;
    #[async_trait]
    impl PrivateHttpResponder for SlowPrivateHandler {
        async fn invoke(&self, _: PrivateHttpRequest) -> Result<PrivateHttpResponse, VmError> {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(PrivateHttpResponse {
                status: StatusCode::NO_CONTENT,
                headers: HeaderMap::new(),
                body: Bytes::new(),
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
            _: Uuid,
        ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError> {
            self.provider
                .provision(gateway_vm_spec(route))
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)
        }
    }

    fn gateway_vm_spec(route: &GatewayRouteBinding) -> VmSpec {
        VmSpec {
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
        )
        .with_dispatcher_upstream(String::from("127.0.0.1:19090"))
        .with_configuration_template(caddy_template());
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
        let loaded = admin.0.lock().expect("lock");
        assert_eq!(loaded.len(), 1);
        assert!(
            String::from_utf8_lossy(&loaded[0]).contains("127.0.0.1:19090"),
            "the derived Caddy configuration targets only the configured private dispatcher"
        );
        drop(loaded);
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
        )
        .with_configuration_template(caddy_template());
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
    async fn inbound_secret_is_constant_time_checked_and_rewritten_before_vm_delivery() {
        let capture = Arc::new(HeaderCapture(Mutex::new(None)));
        let dispatcher = GatewayDispatcher::new(Resolver(route()), Arc::clone(&capture), Recorder)
            .with_inbound_secret_resolver(Arc::new(InboundRules));
        let mut valid = request("/gateway/echo");
        valid.headers.insert(
            "x-hook-secret",
            HeaderValue::from_static("gateway-secret-sentinel"),
        );
        assert_eq!(
            dispatcher.dispatch(valid).await.response.status,
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            capture.0.lock().expect("capture").as_ref(),
            Some(&HeaderValue::from_static(
                "heph-placeholder:v1:gateway-proof"
            ))
        );
        let mut invalid = request("/gateway/echo");
        invalid
            .headers
            .insert("x-hook-secret", HeaderValue::from_static("wrong-secret"));
        assert_eq!(
            dispatcher.dispatch(invalid).await.response.status,
            StatusCode::NOT_FOUND
        );
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

    #[tokio::test]
    async fn timeout_cleans_up_the_one_shot_vm_after_the_request_future_is_cancelled() {
        let route = route();
        let provider =
            FakeProvider::new().with_private_http_responder(Arc::new(SlowPrivateHandler));
        let handler = PrivateHttpVmGatewayHandler::new(FakeGatewayLauncher {
            provider: provider.clone(),
        });
        let dispatcher = GatewayDispatcher::new(Resolver(route.clone()), handler, Recorder);
        assert_eq!(
            dispatcher
                .dispatch(request("/gateway/echo"))
                .await
                .response
                .status,
            StatusCode::GATEWAY_TIMEOUT
        );
        // Provisioning the same deterministic VM ID succeeds only after the
        // detached cleanup task has destroyed the timed-out instance.
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match provider.provision(gateway_vm_spec(&route)).await {
                    Ok(instance) => {
                        instance.destroy().await.expect("destroy replacement VM");
                        return;
                    }
                    Err(VmError::AlreadyExists(_)) => tokio::task::yield_now().await,
                    Err(error) => panic!("unexpected replacement provisioning error: {error}"),
                }
            }
        })
        .await
        .expect("timed-out gateway VM was cleaned up");
    }
}
