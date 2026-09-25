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
    body::{Body, Bytes, to_bytes},
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderName, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::any,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use gateway_domain::HttpMethod;
use identity_domain::RequestId;
use release_artifact_store::LocalArtifactStore;
use release_domain::{
    ContentHash, UiInstallationGenerationId, ui::UiCachePolicy, ui_browser::UiBrowserSessionSecret,
};
use release_service::ui_browser_host::{
    UI_CHILD_COOKIE, UiGenerationHost, UiNamespace, UiPublicPort,
};
use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiBrowserHttpRequest, UiBrowserHttpServingProjection,
    UiGatewayRequestProjection, UiGenerationHostResolver, UiHostLookupError, UiServingError,
    UiServingProjection, UiStaticArtifactProjection,
};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use thiserror::Error;
use tokio::sync::Semaphore;

use crate::{
    ui_audit::{UiAuditRecorder, correlation_id, reason_for_content_error, verified_context},
    ui_origin_config::UiOriginConfig,
};

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

/// Registers the content route behind the exact generation namespace. The
/// bootstrap router owns `/_heph/bootstrap`; this router owns release paths.
pub fn router(state: Arc<UiContentState>) -> Router {
    Router::new()
        .fallback(any(content_request))
        .with_state(state)
}

async fn content_request(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiContentState>>,
    request: Request<Body>,
) -> Response {
    // Correlation is allocated before loopback, host, cookie, or syntax checks.
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Content,
                release_service::UiRequestAuditReason::Unauthorized,
                error_response(StatusCode::FORBIDDEN, "ui_forbidden"),
            )
            .await;
    }
    match tokio::time::timeout(state.deadline, serve(state.clone(), request, request_id)).await {
        Ok(Ok(response)) => response,
        Ok(Err(error)) => {
            state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    reason_for_content_error(error),
                    error.into_response(),
                )
                .await
        }
        Err(_) => {
            state
                .audit
                .undetermined(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    release_service::UiRequestAuditContext::anonymous(),
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "ui_unavailable"),
                )
                .await
        }
    }
}

// The ordered checks in this handler mirror the serving contract: authority,
// authentication, origin, gateway/static dispatch, then response auditing.
#[allow(clippy::too_many_lines)]
async fn serve(
    state: Arc<UiContentState>,
    request: Request<Body>,
    request_id: RequestId,
) -> Result<Response, UiContentError> {
    let authority = request_authority(&request)?;
    let host = parse_and_resolve_host(&state, authority).await?;
    let http_request = UiHttpRequest::parse(&request)?;
    require_unsafe_origin(
        &request,
        &host,
        http_request.method,
        &state.namespace,
        state.public_port,
    )?;
    let child_secret = parse_child_cookie(request.headers())?;
    let authority_request =
        UiBrowserHttpRequest::new(http_request.method, http_request.path.as_str())
            .map_err(|_| UiContentError::InvalidRequest)?;
    let projection = state
        .authority
        .authenticate_and_project_http(
            request_id,
            child_secret,
            host.generation_id(),
            authority_request,
        )
        .await
        .map_err(map_serving_error)?;
    match projection {
        UiServingProjection::Redirect { context, location } => {
            let result = redirect_response(
                &location,
                http_request.query.as_deref(),
                &state.platform_csp,
            );
            match result {
                Ok(response) => Ok(state
                    .audit
                    .allowed(
                        request_id,
                        release_service::UiRequestAuditSurface::Content,
                        verified_context(&context),
                        release_service::UiRequestAuditOutcome::Succeeded,
                        release_service::UiRequestAuditReason::None,
                        response,
                    )
                    .await),
                Err(error) => Ok(state
                    .audit
                    .failed(
                        request_id,
                        release_service::UiRequestAuditSurface::Content,
                        verified_context(&context),
                        reason_for_content_error(error),
                        error.into_response(),
                    )
                    .await),
            }
        }
        UiServingProjection::Static { context, artifact } => {
            let context_for_audit = verified_context(&context);
            let request_headers = StaticRequestHeaders::from_request(&request);
            match serve_static(&state, &request_headers, &http_request, artifact).await {
                Ok(response) => Ok(state
                    .audit
                    .allowed(
                        request_id,
                        release_service::UiRequestAuditSurface::Static,
                        context_for_audit,
                        release_service::UiRequestAuditOutcome::Succeeded,
                        release_service::UiRequestAuditReason::None,
                        response,
                    )
                    .await),
                Err(error) => Ok(state
                    .audit
                    .failed(
                        request_id,
                        release_service::UiRequestAuditSurface::Static,
                        context_for_audit,
                        reason_for_content_error(error),
                        error.into_response(),
                    )
                    .await),
            }
        }
        UiServingProjection::Gateway {
            context,
            request: gateway_request,
        } => {
            let context_for_audit = verified_context(&context);
            let surface = match gateway_request.kind {
                release_service::UiGatewayRequestKind::Managed => {
                    release_service::UiRequestAuditSurface::Managed
                }
                release_service::UiGatewayRequestKind::Api => {
                    release_service::UiRequestAuditSurface::Api
                }
            };
            let result = async {
                let headers = sanitize_guest_request_headers(request.headers().clone());
                let body = to_bytes(request.into_body(), MAX_UI_REQUEST_BODY_BYTES)
                    .await
                    .map_err(|_| UiContentError::BodyTooLarge)?;
                let authority = UiGatewayDispatchAuthority {
                    request_id,
                    child_session_id: context.session_id,
                    actor_id: context.actor_id,
                    organization_id: context.organization_id,
                    installation_id: context.installation_id,
                    generation_id: context.generation_id,
                    request: gateway_request,
                };
                let dispatch = state
                    .gateway
                    .dispatch_ui_detailed(
                        authority,
                        http_request.method,
                        http_request.path_and_query(),
                        headers,
                        body,
                    )
                    .await?;
                let disposition = dispatch.disposition;
                let response = gateway_response(dispatch.response, &state.platform_csp);
                Ok::<_, UiContentError>((disposition, response))
            }
            .await;
            match result {
                Ok((disposition, Ok(response))) => match gateway_audit_disposition(disposition) {
                    GatewayAuditDisposition::Denied(reason) => Ok(state
                        .audit
                        .denial_with_context(
                            request_id,
                            surface,
                            context_for_audit,
                            reason,
                            response,
                        )
                        .await),
                    GatewayAuditDisposition::Failed(reason) => Ok(state
                        .audit
                        .failed(request_id, surface, context_for_audit, reason, response)
                        .await),
                    GatewayAuditDisposition::Unknown => Ok(state
                        .audit
                        .undetermined(request_id, surface, context_for_audit, response)
                        .await),
                    GatewayAuditDisposition::Succeeded => Ok(state
                        .audit
                        .allowed(
                            request_id,
                            surface,
                            context_for_audit,
                            release_service::UiRequestAuditOutcome::Succeeded,
                            release_service::UiRequestAuditReason::None,
                            response,
                        )
                        .await),
                },
                Ok((disposition, Err(error))) => match gateway_audit_disposition(disposition) {
                    GatewayAuditDisposition::Unknown => Ok(state
                        .audit
                        .undetermined(
                            request_id,
                            surface,
                            context_for_audit,
                            error.into_response(),
                        )
                        .await),
                    GatewayAuditDisposition::Denied(reason) => Ok(state
                        .audit
                        .denial_with_context(
                            request_id,
                            surface,
                            context_for_audit,
                            reason,
                            error.into_response(),
                        )
                        .await),
                    GatewayAuditDisposition::Failed(reason) => Ok(state
                        .audit
                        .failed(
                            request_id,
                            surface,
                            context_for_audit,
                            reason,
                            error.into_response(),
                        )
                        .await),
                    GatewayAuditDisposition::Succeeded => Ok(state
                        .audit
                        .failed(
                            request_id,
                            surface,
                            context_for_audit,
                            release_service::UiRequestAuditReason::UpstreamFailure,
                            error.into_response(),
                        )
                        .await),
                },
                Err(error) => Ok(state
                    .audit
                    .failed(
                        request_id,
                        surface,
                        context_for_audit,
                        reason_for_content_error(error),
                        error.into_response(),
                    )
                    .await),
            }
        }
    }
}

enum GatewayAuditDisposition {
    Denied(release_service::UiRequestAuditReason),
    Failed(release_service::UiRequestAuditReason),
    Unknown,
    Succeeded,
}

const fn gateway_audit_disposition(
    disposition: gateway_edge::UiDispatchDisposition,
) -> GatewayAuditDisposition {
    use gateway_edge::{GatewayInvocationOutcome, UiDispatchDisposition};
    match disposition {
        UiDispatchDisposition::ProviderDenied => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unauthorized)
        }
        UiDispatchDisposition::ProviderNotFound => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::NotFound)
        }
        UiDispatchDisposition::ProviderUnavailable => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unavailable)
        }
        UiDispatchDisposition::StructuralInvalid => {
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::InvalidInput)
        }
        UiDispatchDisposition::AcceptedUiFailure => {
            GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::Unavailable)
        }
        UiDispatchDisposition::Admitted {
            completion_persisted: false,
            ..
        }
        | UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::TimedOut,
            completion_persisted: true,
        } => GatewayAuditDisposition::Unknown,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Completed,
            completion_persisted: true,
        } => GatewayAuditDisposition::Succeeded,
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Rejected,
            completion_persisted: true,
        } => GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::Unauthorized),
        UiDispatchDisposition::Admitted {
            outcome: GatewayInvocationOutcome::Failed,
            completion_persisted: true,
        } => {
            GatewayAuditDisposition::Failed(release_service::UiRequestAuditReason::UpstreamFailure)
        }
    }
}

fn redirect_response(
    location: &release_service::UiBrowserHttpPath,
    query: Option<&str>,
    platform_csp: &HeaderValue,
) -> Result<Response, UiContentError> {
    let location = query.map_or_else(
        || location.as_str().to_owned(),
        |query| format!("{}?{query}", location.as_str()),
    );
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::TEMPORARY_REDIRECT;
    response.headers_mut().insert(
        header::LOCATION,
        HeaderValue::from_str(&location).map_err(|_| UiContentError::Unavailable)?,
    );
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_platform_csp(response.headers_mut(), platform_csp);
    Ok(response)
}

const fn map_serving_error(error: UiServingError) -> UiContentError {
    match error {
        UiServingError::Unauthenticated => UiContentError::Unauthenticated,
        UiServingError::Unavailable => UiContentError::Unavailable,
    }
}

async fn parse_and_resolve_host(
    state: &UiContentState,
    authority: String,
) -> Result<UiGenerationHost, UiContentError> {
    let host = UiGenerationHost::parse(&authority, &state.namespace, state.public_port)
        .map_err(|_| UiContentError::NotFound)?;
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
        .map_err(|error| match error {
            UiHostLookupError::Unavailable => UiContentError::Unavailable,
        })? {
        Some(ActiveUiGenerationHost { generation_id }) if generation_id == host.generation_id() => {
            Ok(host)
        }
        Some(_) | None => Err(UiContentError::NotFound),
    }
}

fn request_authority(request: &Request<Body>) -> Result<String, UiContentError> {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().get_all(header::HOST).iter().count() != 1
    {
        return Err(UiContentError::InvalidRequest);
    }
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(UiContentError::InvalidRequest)
}

fn require_unsafe_origin(
    request: &Request<Body>,
    host: &UiGenerationHost,
    method: HttpMethod,
    namespace: &UiNamespace,
    public_port: UiPublicPort,
) -> Result<(), UiContentError> {
    let origin_count = request.headers().get_all(header::ORIGIN).iter().count();
    if origin_count == 0 && matches!(method, HttpMethod::Get | HttpMethod::Head) {
        return Ok(());
    }
    if origin_count != 1 {
        return Err(UiContentError::InvalidRequest);
    }
    let expected = format!("https://{}", host.authority(namespace, public_port));
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .ok_or(UiContentError::InvalidRequest)?;
    if origin != expected {
        return Err(UiContentError::InvalidRequest);
    }
    if !matches!(method, HttpMethod::Get | HttpMethod::Head)
        && request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value.as_bytes() != b"same-origin")
    {
        return Err(UiContentError::InvalidRequest);
    }
    Ok(())
}

fn parse_child_cookie(headers: &HeaderMap) -> Result<UiBrowserSessionSecret, UiContentError> {
    let mut found = None;
    for value in &headers.get_all(header::COOKIE) {
        let value = value
            .to_str()
            .map_err(|_| UiContentError::Unauthenticated)?;
        for pair in value.split(';') {
            let pair = pair.trim();
            let Some((name, encoded)) = pair.split_once('=') else {
                return Err(UiContentError::Unauthenticated);
            };
            if name != UI_CHILD_COOKIE {
                continue;
            }
            if found.is_some() || !is_secret_text(encoded) {
                return Err(UiContentError::Unauthenticated);
            }
            let bytes = URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| UiContentError::Unauthenticated)?;
            let bytes: [u8; 32] = bytes
                .try_into()
                .map_err(|_| UiContentError::Unauthenticated)?;
            found = Some(UiBrowserSessionSecret::from_bytes(bytes));
        }
    }
    found.ok_or(UiContentError::Unauthenticated)
}

fn is_secret_text(value: &str) -> bool {
    value.len() == 43
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

async fn serve_static(
    state: &UiContentState,
    request_headers: &StaticRequestHeaders,
    http_request: &UiHttpRequest,
    artifact: UiStaticArtifactProjection,
) -> Result<Response, UiContentError> {
    if !matches!(http_request.method, HttpMethod::Get | HttpMethod::Head) {
        return Err(UiContentError::NotFound);
    }
    let cache_policy = artifact.cache_policy;
    if artifact.size_bytes > state.max_static_bytes {
        return Err(UiContentError::Unavailable);
    }
    let permit = state
        .static_read_permits
        .clone()
        .acquire_owned()
        .await
        .map_err(|_| UiContentError::Unavailable)?;
    let store = Arc::clone(&state.artifacts);
    let read = tokio::time::timeout(
        state.deadline,
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            store.read_verified(
                artifact.storage_key,
                artifact.content_hash,
                artifact.size_bytes,
                artifact.size_bytes,
            )
        }),
    )
    .await
    .map_err(|_| UiContentError::Unavailable)?
    .map_err(|_| UiContentError::Unavailable)?
    .map_err(|_| UiContentError::Unavailable)?;
    static_response(
        request_headers,
        http_request,
        artifact.content_hash,
        artifact.media_type.as_str(),
        cache_policy,
        &state.platform_csp,
        read,
    )
}

#[derive(Debug, Default)]
struct StaticRequestHeaders {
    if_none_match: Option<String>,
    range: Option<String>,
    if_range: Option<String>,
}

impl StaticRequestHeaders {
    fn from_request(request: &Request<Body>) -> Self {
        let header_value = |name| {
            request
                .headers()
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned)
        };
        Self {
            if_none_match: header_value(header::IF_NONE_MATCH),
            range: header_value(header::RANGE),
            if_range: header_value(header::IF_RANGE),
        }
    }
}

fn static_response(
    request_headers: &StaticRequestHeaders,
    http_request: &UiHttpRequest,
    hash: ContentHash,
    media_type: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
    bytes: Vec<u8>,
) -> Result<Response, UiContentError> {
    let etag = format!("\"{}\"", hex_bytes(hash.as_bytes()));
    if request_headers
        .if_none_match
        .as_deref()
        .is_some_and(|value| value == etag)
    {
        let mut response = Response::new(Body::empty());
        *response.status_mut() = StatusCode::NOT_MODIFIED;
        common_static_headers(
            response.headers_mut(),
            media_type,
            &etag,
            cache_policy,
            platform_csp,
        );
        return Ok(response);
    }
    let range = match (&request_headers.range, &request_headers.if_range) {
        (Some(_), Some(if_range)) if if_range != &etag => None,
        (Some(value), _) => match parse_single_range(value, bytes.len() as u64) {
            Ok(range) => Some(range),
            Err(UiContentError::RangeNotSatisfiable) => {
                return Ok(range_not_satisfiable(
                    bytes.len() as u64,
                    media_type,
                    &etag,
                    cache_policy,
                    platform_csp,
                ));
            }
            Err(error) => return Err(error),
        },
        (None, _) => None,
    };
    let (status, body, content_range) = match range {
        Some((start, end)) => (
            StatusCode::PARTIAL_CONTENT,
            bytes[usize::try_from(start).map_err(|_| UiContentError::Unavailable)?
                ..=usize::try_from(end).map_err(|_| UiContentError::Unavailable)?]
                .to_vec(),
            Some(format!("bytes {start}-{end}/{}", bytes.len())),
        ),
        None => (StatusCode::OK, bytes, None),
    };
    let body_len = body.len();
    let is_head = http_request.method == HttpMethod::Head;
    let mut response = if is_head {
        Response::new(Body::empty())
    } else {
        Response::new(Body::from(body))
    };
    *response.status_mut() = status;
    common_static_headers(
        response.headers_mut(),
        media_type,
        &etag,
        cache_policy,
        platform_csp,
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&body_len.to_string()).map_err(|_| UiContentError::Unavailable)?,
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some(content_range) = content_range {
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&content_range).map_err(|_| UiContentError::Unavailable)?,
        );
    }
    Ok(response)
}

fn range_not_satisfiable(
    length: u64,
    media_type: &str,
    etag: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = StatusCode::RANGE_NOT_SATISFIABLE;
    common_static_headers(
        response.headers_mut(),
        media_type,
        etag,
        cache_policy,
        platform_csp,
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    response.headers_mut().insert(
        header::CONTENT_RANGE,
        HeaderValue::from_str(&format!("bytes */{length}")).expect("bounded content range header"),
    );
    response
}

fn common_static_headers(
    headers: &mut HeaderMap,
    media_type: &str,
    etag: &str,
    cache_policy: UiCachePolicy,
    platform_csp: &HeaderValue,
) {
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(media_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::ETAG, HeaderValue::from_str(etag).expect("hex ETag"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    if matches!(cache_policy, UiCachePolicy::NoStore) {
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
    apply_platform_csp(headers, platform_csp);
}

fn parse_single_range(value: &str, length: u64) -> Result<(u64, u64), UiContentError> {
    if length == 0 || !value.starts_with("bytes=") {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let range = &value[6..];
    if range.is_empty() || range.contains(',') {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let (start, end) = range
        .split_once('-')
        .ok_or(UiContentError::RangeNotSatisfiable)?;
    if start.is_empty() {
        let suffix = end
            .parse::<u64>()
            .map_err(|_| UiContentError::RangeNotSatisfiable)?;
        if suffix == 0 {
            return Err(UiContentError::RangeNotSatisfiable);
        }
        let start = length.saturating_sub(suffix);
        return Ok((start, length - 1));
    }
    let start = start
        .parse::<u64>()
        .map_err(|_| UiContentError::RangeNotSatisfiable)?;
    if start >= length {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    let end = if end.is_empty() {
        length - 1
    } else {
        end.parse::<u64>()
            .map_err(|_| UiContentError::RangeNotSatisfiable)?
            .min(length - 1)
    };
    if end < start {
        return Err(UiContentError::RangeNotSatisfiable);
    }
    Ok((start, end))
}

fn gateway_response(
    response: UiGatewayResponse,
    platform_csp: &HeaderValue,
) -> Result<Response, UiContentError> {
    if response.body.len() > MAX_UI_RESPONSE_BODY_BYTES {
        return Err(UiContentError::BodyTooLarge);
    }
    if response
        .headers
        .get_all(header::SET_COOKIE)
        .iter()
        .next()
        .is_some()
    {
        return Err(UiContentError::GuestSetCookie);
    }
    if response.headers.contains_key(header::LOCATION)
        || response
            .headers
            .contains_key(HeaderName::from_static("refresh"))
    {
        return Err(UiContentError::GuestRedirect);
    }
    let mut headers = sanitize_guest_response_headers(response.headers);
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    apply_platform_csp(&mut headers, platform_csp);
    let mut result = Response::new(Body::from(response.body));
    *result.status_mut() = response.status;
    *result.headers_mut() = headers;
    Ok(result)
}

fn sanitize_guest_request_headers(mut headers: HeaderMap) -> HeaderMap {
    for name in [
        header::AUTHORIZATION,
        header::COOKIE,
        header::CONNECTION,
        header::PROXY_AUTHORIZATION,
        header::PROXY_AUTHENTICATE,
        header::TE,
        header::TRAILER,
        header::TRANSFER_ENCODING,
        header::UPGRADE,
        header::HOST,
        header::ORIGIN,
        header::REFERER,
    ] {
        headers.remove(name);
    }
    for name in ["x-forwarded-for", "x-forwarded-host", "x-forwarded-proto"] {
        headers.remove(name);
    }
    headers
}

fn sanitize_guest_response_headers(mut headers: HeaderMap) -> HeaderMap {
    for name in [
        header::SET_COOKIE,
        header::AUTHORIZATION,
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        header::CONNECTION,
        header::TRANSFER_ENCODING,
        header::CONTENT_SECURITY_POLICY,
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        header::ACCESS_CONTROL_ALLOW_CREDENTIALS,
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        header::ACCESS_CONTROL_ALLOW_METHODS,
        header::ACCESS_CONTROL_EXPOSE_HEADERS,
        header::LOCATION,
        HeaderName::from_static("refresh"),
    ] {
        headers.remove(name);
    }
    headers
}

fn apply_platform_csp(headers: &mut HeaderMap, platform_csp: &HeaderValue) {
    headers.insert(header::CONTENT_SECURITY_POLICY, platform_csp.clone());
}

fn platform_csp_value(platform_origin: &str) -> Result<HeaderValue, UiContentError> {
    HeaderValue::from_str(&format!(
        "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; worker-src 'none'; object-src 'none'; frame-src 'none'; frame-ancestors {platform_origin}; connect-src 'self'; form-action 'none'; base-uri 'none'"
    ))
    .map_err(|_| UiContentError::Unavailable)
}

fn map_method(method: &http::Method) -> Option<HttpMethod> {
    Some(match method.as_str() {
        "GET" => HttpMethod::Get,
        "HEAD" => HttpMethod::Head,
        "POST" => HttpMethod::Post,
        "PUT" => HttpMethod::Put,
        "PATCH" => HttpMethod::Patch,
        "DELETE" => HttpMethod::Delete,
        "OPTIONS" => HttpMethod::Options,
        _ => return None,
    })
}

fn hex_bytes(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(64);
    for byte in bytes {
        result.push(HEX[usize::from(byte >> 4)] as char);
        result.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    result
}

/// Redacted content errors. Every variant maps to generic no-store HTTP.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiContentError {
    /// Request syntax, path, host, cookie, or range was invalid.
    #[error("UI request is invalid")]
    InvalidRequest,
    /// No active generation or declaration matched.
    #[error("UI resource was not found")]
    NotFound,
    /// Child authentication or current authorization failed.
    #[error("UI request is unauthenticated")]
    Unauthenticated,
    /// A bounded body/range limit was exceeded.
    #[error("UI request is too large")]
    BodyTooLarge,
    /// A single range could not be satisfied.
    #[error("UI byte range is not satisfiable")]
    RangeNotSatisfiable,
    /// The guest attempted to set a browser cookie.
    #[error("UI guest response contained a cookie")]
    GuestSetCookie,
    /// The guest attempted to redirect the browser outside the declared UI path.
    #[error("UI guest response contained a redirect")]
    GuestRedirect,
    /// Adapter, artifact store, or gateway was unavailable.
    #[error("UI serving is unavailable")]
    Unavailable,
}

impl IntoResponse for UiContentError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::InvalidRequest => StatusCode::BAD_REQUEST,
            Self::RangeNotSatisfiable => StatusCode::RANGE_NOT_SATISFIABLE,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::BodyTooLarge => StatusCode::PAYLOAD_TOO_LARGE,
            Self::GuestSetCookie | Self::GuestRedirect => StatusCode::BAD_GATEWAY,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
        };
        error_response(status, "ui_unavailable")
    }
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: &'static str,
}

fn error_response(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, axum::Json(ErrorBody { error: code })).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::to_bytes;
    use release_domain::{
        ContentHash, ReleaseArtifactId, UiInstallationId,
        ui::UiMediaType,
        ui_browser::{UiBrowserRoute, UiBrowserSessionId},
    };
    use release_service::UiBrowserSessionContext;
    use std::{
        fs,
        os::unix::fs::PermissionsExt,
        path::PathBuf,
        sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use time::OffsetDateTime;
    use tokio::sync::Mutex;
    use tower::ServiceExt;
    use uuid::Uuid;

    #[test]
    fn canonical_request_keeps_query_opaque() {
        let request = Request::builder()
            .method("GET")
            .uri("/docs/app.js?x=%2F%2F&heph_theme=dark")
            .header(
                header::HOST,
                "g-0123456789abcdef0123456789abcdef.ui.app.example",
            )
            .body(Body::empty())
            .expect("request");
        let parsed = UiHttpRequest::parse(&request).expect("canonical request");
        assert_eq!(parsed.path.as_str(), "/docs/app.js");
        assert_eq!(parsed.query.as_deref(), Some("x=%2F%2F&heph_theme=dark"));
    }

    #[test]
    fn canonical_alias_redirect_is_temporary_and_preserves_opaque_query() {
        let location =
            release_service::UiBrowserHttpPath::parse("/docs/index.html").expect("entrypoint");
        let csp = platform_csp_value("https://platform.example").expect("CSP");
        let response = redirect_response(&location, Some(""), &csp).expect("redirect");
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(response.headers()[header::LOCATION], "/docs/index.html?");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn malformed_path_and_duplicate_host_fail_closed() {
        for uri in [
            "/docs/../secret",
            "/docs/%2e%2e/secret",
            "/docs//x",
            "/docs/",
        ] {
            let request = Request::builder()
                .method("GET")
                .uri(uri)
                .header(
                    header::HOST,
                    "g-0123456789abcdef0123456789abcdef.ui.app.example",
                )
                .body(Body::empty())
                .expect("request");
            assert!(UiHttpRequest::parse(&request).is_err(), "{uri}");
        }
        let request = Request::builder()
            .method("GET")
            .uri("/docs/index.html")
            .header(header::HOST, "one.example")
            .header(header::HOST, "two.example")
            .body(Body::empty())
            .expect("request");
        assert!(UiHttpRequest::parse(&request).is_err());
    }

    #[test]
    fn cookie_requires_one_exact_unpadded_secret() {
        let value = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let mut headers = HeaderMap::new();
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{UI_CHILD_COOKIE}={value}")).expect("cookie header"),
        );
        assert!(parse_child_cookie(&headers).is_ok());
        headers.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "{UI_CHILD_COOKIE}={value}; {UI_CHILD_COOKIE}={value}"
            ))
            .expect("duplicate cookie header"),
        );
        assert!(parse_child_cookie(&headers).is_err());
    }

    #[test]
    fn ranges_are_single_and_bounded() {
        assert_eq!(parse_single_range("bytes=2-4", 8), Ok((2, 4)));
        assert_eq!(parse_single_range("bytes=5-", 8), Ok((5, 7)));
        assert_eq!(parse_single_range("bytes=-2", 8), Ok((6, 7)));
        assert!(parse_single_range("bytes=2-4,6-7", 8).is_err());
        assert!(parse_single_range("bytes=9-10", 8).is_err());
    }

    #[test]
    fn response_policy_forces_nosniff_on_guest_success() {
        let response = gateway_response(
            UiGatewayResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from_static(b"ok"),
            },
            &platform_csp_value("https://platform.example").expect("CSP"),
        )
        .expect("response");
        assert_eq!(
            response.headers()[header::X_CONTENT_TYPE_OPTIONS],
            "nosniff"
        );
    }

    #[test]
    fn response_policy_rejects_guest_set_cookie() {
        let mut headers = HeaderMap::new();
        headers.insert(header::SET_COOKIE, HeaderValue::from_static("guest=x"));
        let response = UiGatewayResponse {
            status: StatusCode::OK,
            headers,
            body: Bytes::new(),
        };
        assert!(matches!(
            gateway_response(
                response,
                &platform_csp_value("https://platform.example").expect("CSP"),
            ),
            Err(UiContentError::GuestSetCookie)
        ));
    }

    #[test]
    fn unsafe_requests_require_exact_generation_origin() {
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        let port = UiPublicPort::https_default();
        let host = UiGenerationHost::from_generation_id(
            release_domain::UiInstallationGenerationId::from_uuid(
                uuid::Uuid::parse_str("01234567-89ab-cdef-0123-456789abcdef").expect("UUID"),
            ),
        );
        let expected = format!("https://{}", host.authority(&namespace, port));
        let request = Request::builder()
            .method("POST")
            .uri("/service/api")
            .header(header::HOST, host.authority(&namespace, port))
            .header(header::ORIGIN, expected)
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty())
            .expect("request");
        assert!(require_unsafe_origin(&request, &host, HttpMethod::Post, &namespace, port).is_ok());
        let foreign = Request::builder()
            .method("POST")
            .uri("/service/api")
            .header(header::HOST, host.authority(&namespace, port))
            .header(header::ORIGIN, "null")
            .body(Body::empty())
            .expect("foreign request");
        assert!(
            require_unsafe_origin(&foreign, &host, HttpMethod::Post, &namespace, port).is_err()
        );
    }

    #[test]
    fn platform_csp_cannot_be_overridden_by_guest_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("default-src *"),
        );
        let sanitized = sanitize_guest_response_headers(headers);
        assert!(sanitized.get(header::CONTENT_SECURITY_POLICY).is_none());
        let mut final_headers = HeaderMap::new();
        let csp = platform_csp_value("https://platform.example").expect("CSP");
        apply_platform_csp(&mut final_headers, &csp);
        assert_eq!(
            final_headers
                .get(header::CONTENT_SECURITY_POLICY)
                .and_then(|value| value.to_str().ok()),
            Some(
                "default-src 'none'; script-src 'self'; style-src 'self'; img-src 'self'; font-src 'self'; worker-src 'none'; object-src 'none'; frame-src 'none'; frame-ancestors https://platform.example; connect-src 'self'; form-action 'none'; base-uri 'none'"
            )
        );
    }

    #[test]
    fn platform_origin_validation_requires_same_site_exact_https_origin() {
        let port = UiPublicPort::https_default();
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        assert!(UiOriginConfig::new(namespace.clone(), port, "https://app.example").is_ok());
        assert!(UiOriginConfig::new(namespace.clone(), port, "https://other.example").is_err());
        assert!(UiOriginConfig::new(namespace.clone(), port, "https://app.example/path").is_err());
        assert!(UiOriginConfig::new(namespace, port, "https://app.example:0443").is_err());
    }

    #[test]
    fn etag_is_lowercase_hex() {
        let mut bytes = [0_u8; 32];
        bytes[1] = 0xab;
        bytes[2] = 0xff;
        assert_eq!(hex_bytes(&bytes), format!("00abff{}", "00".repeat(29)));
    }

    #[derive(Clone)]
    struct FakeHostResolver {
        generation_id: UiInstallationGenerationId,
    }

    #[async_trait]
    impl UiGenerationHostResolver for FakeHostResolver {
        async fn resolve_active_generation_host(
            &self,
            host: UiGenerationHost,
        ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
            Ok(
                (host.generation_id() == self.generation_id).then_some(ActiveUiGenerationHost {
                    generation_id: self.generation_id,
                }),
            )
        }
    }

    struct FakeAuthority {
        calls: AtomicUsize,
        projection: Mutex<Option<UiServingProjection>>,
    }

    #[async_trait]
    impl UiBrowserHttpServingProjection for FakeAuthority {
        async fn authenticate_and_project_http(
            &self,
            _request_id: RequestId,
            _session_secret: UiBrowserSessionSecret,
            _expected_generation_id: UiInstallationGenerationId,
            _request: UiBrowserHttpRequest,
        ) -> Result<UiServingProjection, UiServingError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.projection
                .lock()
                .await
                .clone()
                .ok_or(UiServingError::Unauthenticated)
        }
    }

    struct FakeGateway {
        calls: AtomicUsize,
        captured: Mutex<Option<UiGatewayDispatchAuthority>>,
        response: Mutex<Option<UiGatewayResponse>>,
    }

    struct StartedTimeoutGateway {
        started: Arc<AtomicBool>,
    }

    #[async_trait]
    impl UiGatewayDispatcher for FakeGateway {
        async fn dispatch_ui_detailed(
            &self,
            authority: UiGatewayDispatchAuthority,
            _method: HttpMethod,
            _path_and_query: String,
            headers: HeaderMap,
            _body: Bytes,
        ) -> Result<UiGatewayDispatchResult, UiContentError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert!(headers.get(header::AUTHORIZATION).is_none());
            assert!(headers.get(header::COOKIE).is_none());
            *self.captured.lock().await = Some(authority);
            let response = self
                .response
                .lock()
                .await
                .clone()
                .ok_or(UiContentError::Unavailable)?;
            Ok(UiGatewayDispatchResult {
                response,
                disposition: gateway_edge::UiDispatchDisposition::Admitted {
                    outcome: gateway_edge::GatewayInvocationOutcome::Completed,
                    completion_persisted: true,
                },
            })
        }
    }

    #[async_trait]
    impl UiGatewayDispatcher for StartedTimeoutGateway {
        async fn dispatch_ui_detailed(
            &self,
            _authority: UiGatewayDispatchAuthority,
            _method: HttpMethod,
            _path_and_query: String,
            _headers: HeaderMap,
            _body: Bytes,
        ) -> Result<UiGatewayDispatchResult, UiContentError> {
            self.started.store(true, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(250)).await;
            Err(UiContentError::Unavailable)
        }
    }

    fn context(generation_id: UiInstallationGenerationId) -> UiBrowserSessionContext {
        UiBrowserSessionContext {
            session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
            parent_session_id: identity_domain::BrowserSessionId::from_uuid(Uuid::new_v4()),
            actor_id: identity_domain::UserId::from_uuid(Uuid::new_v4()),
            organization_id: forge_domain::OrganizationId::from_uuid(Uuid::new_v4()),
            installation_id: UiInstallationId::from_uuid(Uuid::new_v4()),
            generation_id,
            route: UiBrowserRoute::parse("schema-ui").expect("route"),
            expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
        }
    }

    fn host_and_state_with_sink(
        authority: Arc<FakeAuthority>,
        artifacts: Arc<LocalArtifactStore>,
        gateway: Arc<dyn UiGatewayDispatcher>,
        generation_id: UiInstallationGenerationId,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> (Arc<UiContentState>, String) {
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        let port = UiPublicPort::https_default();
        let host = UiGenerationHost::from_generation_id(generation_id);
        let state = UiContentState::new(
            Arc::new(FakeHostResolver { generation_id }),
            authority,
            artifacts,
            gateway,
            namespace,
            port,
            "https://app.example",
            audit_sink,
        )
        .expect("state");
        (
            Arc::new(state),
            host.authority(&UiNamespace::parse("ui.app.example").unwrap(), port),
        )
    }

    fn host_and_state(
        authority: Arc<FakeAuthority>,
        artifacts: Arc<LocalArtifactStore>,
        gateway: Arc<FakeGateway>,
        generation_id: UiInstallationGenerationId,
    ) -> (Arc<UiContentState>, String) {
        host_and_state_with_sink(
            authority,
            artifacts,
            gateway,
            generation_id,
            Arc::new(crate::ui_audit::NoopAuditSink),
        )
    }

    fn static_artifact(bytes: &[u8], storage_key: Uuid) -> UiStaticArtifactProjection {
        UiStaticArtifactProjection {
            artifact_id: ReleaseArtifactId::from_uuid(Uuid::new_v4()),
            storage_key,
            content_hash: ContentHash::digest(bytes),
            size_bytes: bytes.len() as u64,
            media_type: UiMediaType::TextPlain,
            cache_policy: UiCachePolicy::NoStore,
        }
    }

    fn temp_store(bytes: &[u8], storage_key: Uuid) -> (Arc<LocalArtifactStore>, PathBuf) {
        let root = std::env::temp_dir().join(format!("heph-ui-handler-{}", Uuid::new_v4()));
        fs::create_dir(&root).expect("artifact root");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).expect("root mode");
        fs::write(root.join(storage_key.simple().to_string()), bytes).expect("artifact");
        (
            Arc::new(LocalArtifactStore::new(root.clone()).expect("store")),
            root,
        )
    }

    fn content_request(
        method: &str,
        uri: &str,
        authority: &str,
        headers: &[(&str, &str)],
        body: Body,
    ) -> Request<Body> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, authority)
            .header(
                header::COOKIE,
                format!("{UI_CHILD_COOKIE}={}", URL_SAFE_NO_PAD.encode([7_u8; 32])),
            );
        for (name, value) in headers {
            builder = builder.header(*name, *value);
        }
        let mut request = builder.body(body).expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        request
    }

    #[tokio::test]
    async fn content_router_audits_verified_static_success_before_bytes() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"audited", key);
        let context = context(generation_id);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(Some(UiServingProjection::Static {
                context,
                artifact: static_artifact(b"audited", key),
            })),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(None),
        });
        let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
        let (state, host) = host_and_state_with_sink(
            authority,
            artifacts,
            gateway,
            generation_id,
            Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
        );
        let request = content_request("GET", "/docs/index.html", &host, &[], Body::empty());
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            to_bytes(response.into_body(), 128).await.expect("body"),
            "audited"
        );
        let events = sink.events.lock().expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].surface(),
            release_service::UiRequestAuditSurface::Static
        );
        assert_eq!(
            events[0].decision(),
            release_service::UiRequestAuditDecision::Allowed
        );
        assert_eq!(
            events[0].outcome(),
            release_service::UiRequestAuditOutcome::Succeeded
        );
        assert!(events[0].context().child_session_id().is_some());
        drop(events);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn content_router_keeps_denial_when_audit_sink_is_unavailable() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"unused", key);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(None),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(None),
        });
        let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
        sink.fail.store(true, Ordering::Relaxed);
        let (state, host) = host_and_state_with_sink(
            authority,
            artifacts,
            gateway,
            generation_id,
            Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
        );
        let request = content_request("GET", "/docs/index.html", &host, &[], Body::empty());
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(sink.events.lock().expect("events").is_empty());
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn content_router_denies_cross_generation_unsafe_origin_before_authority() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"content", key);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(None),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(None),
        });
        let (state, host) = host_and_state(authority.clone(), artifacts, gateway, generation_id);
        let request = content_request(
            "POST",
            "/service/api",
            &host,
            &[("origin", "https://other.ui.app.example")],
            Body::from("body"),
        );
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(authority.calls.load(Ordering::SeqCst), 0);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)] // Covers the complete static response matrix.
    async fn content_router_serves_verified_static_etag_range_head_and_tamper_failure() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let bytes = b"verified-content";
        let (artifacts, root) = temp_store(bytes, key);
        let artifact = static_artifact(bytes, key);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(Some(UiServingProjection::Static {
                context: context(generation_id),
                artifact: artifact.clone(),
            })),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(None),
        });
        let (mut state, host) =
            host_and_state(Arc::clone(&authority), artifacts, gateway, generation_id);
        let etag = format!("\"{}\"", hex_bytes(artifact.content_hash.as_bytes()));
        let request = content_request(
            "GET",
            "/schema-ui/index.txt",
            &host,
            &[("if-none-match", &etag)],
            Body::empty(),
        );
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("304");
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let duplicate_cookie = format!(
            "{UI_CHILD_COOKIE}={}; {UI_CHILD_COOKIE}={}",
            URL_SAFE_NO_PAD.encode([7_u8; 32]),
            URL_SAFE_NO_PAD.encode([8_u8; 32])
        );
        let request = content_request(
            "GET",
            "/schema-ui/index.txt",
            &host,
            &[("cookie", &duplicate_cookie)],
            Body::empty(),
        );
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("duplicate cookie response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let request = content_request(
            "GET",
            "/schema-ui/index.txt",
            &host,
            &[("range", "bytes=1-3"), ("if-range", &etag)],
            Body::empty(),
        );
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("206");
        assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            to_bytes(response.into_body(), 32)
                .await
                .expect("range")
                .as_ref(),
            b"eri"
        );
        let request = content_request("HEAD", "/schema-ui/index.txt", &host, &[], Body::empty());
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("HEAD");
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            to_bytes(response.into_body(), 32)
                .await
                .expect("head")
                .is_empty()
        );
        *authority.projection.lock().await = None;
        let request = content_request(
            "GET",
            "/schema-ui/index.txt",
            &host,
            &[("if-none-match", &etag), ("range", "bytes=0-1")],
            Body::empty(),
        );
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("denial");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        *authority.projection.lock().await = Some(UiServingProjection::Static {
            context: context(generation_id),
            artifact: artifact.clone(),
        });
        Arc::get_mut(&mut state)
            .expect("exclusive state")
            .max_static_bytes = 2;
        let request = content_request("GET", "/schema-ui/index.txt", &host, &[], Body::empty());
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("size response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        Arc::get_mut(&mut state)
            .expect("exclusive state")
            .max_static_bytes = DEFAULT_MAX_STATIC_BYTES;
        fs::write(root.join(key.simple().to_string()), b"tampered").expect("tamper");
        let request = content_request("GET", "/schema-ui/index.txt", &host, &[], Body::empty());
        let response = router(state)
            .oneshot(request)
            .await
            .expect("tamper response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn content_router_preserves_child_only_gateway_authority_and_rejects_guest_cookie() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"unused", key);
        let request_projection = UiGatewayRequestProjection {
            kind: release_service::UiGatewayRequestKind::Api,
            path: release_service::UiBrowserHttpPath::parse("/service/api").expect("path"),
            method: HttpMethod::Post,
        };
        let context = context(generation_id);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(Some(UiServingProjection::Gateway {
                context: context.clone(),
                request: request_projection,
            })),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(Some(UiGatewayResponse {
                status: StatusCode::OK,
                headers: HeaderMap::new(),
                body: Bytes::from_static(b"ok"),
            })),
        });
        let (state, host) = host_and_state(
            Arc::clone(&authority),
            artifacts,
            Arc::clone(&gateway),
            generation_id,
        );
        let request = content_request(
            "POST",
            "/service/api?x=%2F%2F",
            &host,
            &[
                ("origin", &format!("https://{host}")),
                ("authorization", "bad"),
            ],
            Body::from("payload"),
        );
        let response = router(Arc::clone(&state))
            .oneshot(request)
            .await
            .expect("gateway");
        assert_eq!(response.status(), StatusCode::OK);
        let captured = gateway.captured.lock().await.take().expect("authority");
        assert_eq!(captured.child_session_id, context.session_id);
        assert_eq!(captured.generation_id, generation_id);
        let response_headers =
            HeaderMap::from_iter([(header::SET_COOKIE, HeaderValue::from_static("guest=x"))]);
        *gateway.response.lock().await = Some(UiGatewayResponse {
            status: StatusCode::OK,
            headers: response_headers,
            body: Bytes::new(),
        });
        let request = content_request(
            "POST",
            "/service/api",
            &host,
            &[("origin", &format!("https://{host}"))],
            Body::empty(),
        );
        let response = router(state)
            .oneshot(request)
            .await
            .expect("guest cookie response");
        assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn content_router_authorizes_base_redirect_before_artifact_or_guest_access() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"must not be read", key);
        let context = context(generation_id);
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(Some(UiServingProjection::Redirect {
                context,
                location: release_service::UiBrowserHttpPath::parse("/docs/index.html")
                    .expect("entrypoint"),
            })),
        });
        let gateway = Arc::new(FakeGateway {
            calls: AtomicUsize::new(0),
            captured: Mutex::new(None),
            response: Mutex::new(None),
        });
        let (state, host) = host_and_state(
            Arc::clone(&authority),
            artifacts,
            Arc::clone(&gateway),
            generation_id,
        );
        let request = content_request("GET", "/docs?x=%2F%2F&empty=", &host, &[], Body::empty());
        let response = router(state).oneshot(request).await.expect("redirect");
        assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(
            response.headers()[header::LOCATION],
            "/docs/index.html?x=%2F%2F&empty="
        );
        assert_eq!(authority.calls.load(Ordering::SeqCst), 1);
        assert_eq!(gateway.calls.load(Ordering::SeqCst), 0);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[tokio::test]
    async fn gateway_started_before_deadline_is_audited_as_unknown() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let key = Uuid::new_v4();
        let (artifacts, root) = temp_store(b"unused", key);
        let request_projection = UiGatewayRequestProjection {
            kind: release_service::UiGatewayRequestKind::Api,
            path: release_service::UiBrowserHttpPath::parse("/service/api").expect("path"),
            method: HttpMethod::Post,
        };
        let authority = Arc::new(FakeAuthority {
            calls: AtomicUsize::new(0),
            projection: Mutex::new(Some(UiServingProjection::Gateway {
                context: context(generation_id),
                request: request_projection,
            })),
        });
        let started = Arc::new(AtomicBool::new(false));
        let gateway: Arc<dyn UiGatewayDispatcher> = Arc::new(StartedTimeoutGateway {
            started: Arc::clone(&started),
        });
        let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
        let (mut state, host) = host_and_state_with_sink(
            authority,
            artifacts,
            gateway,
            generation_id,
            Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
        );
        Arc::get_mut(&mut state).expect("unique state").deadline = Duration::from_millis(100);
        let request = content_request(
            "POST",
            "/service/api",
            &host,
            &[("origin", &format!("https://{host}"))],
            Body::empty(),
        );
        let response = router(state)
            .oneshot(request)
            .await
            .expect("timeout response");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(started.load(Ordering::SeqCst));
        let events = sink.events.lock().expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].decision(),
            release_service::UiRequestAuditDecision::Undetermined
        );
        assert_eq!(
            events[0].outcome(),
            release_service::UiRequestAuditOutcome::Unknown
        );
        drop(events);
        fs::remove_dir_all(root).expect("cleanup");
    }

    #[test]
    fn gateway_denial_and_guest_forbidden_have_distinct_audit_classes() {
        assert!(matches!(
            gateway_audit_disposition(gateway_edge::UiDispatchDisposition::ProviderDenied),
            GatewayAuditDisposition::Denied(release_service::UiRequestAuditReason::Unauthorized)
        ));
        assert!(matches!(
            gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
                outcome: gateway_edge::GatewayInvocationOutcome::Completed,
                completion_persisted: true,
            }),
            GatewayAuditDisposition::Succeeded
        ));
    }

    #[test]
    fn gateway_timeout_and_completion_store_failure_are_unknown() {
        assert!(matches!(
            gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
                outcome: gateway_edge::GatewayInvocationOutcome::TimedOut,
                completion_persisted: true,
            }),
            GatewayAuditDisposition::Unknown
        ));
        assert!(matches!(
            gateway_audit_disposition(gateway_edge::UiDispatchDisposition::Admitted {
                outcome: gateway_edge::GatewayInvocationOutcome::Completed,
                completion_persisted: false,
            }),
            GatewayAuditDisposition::Unknown
        ));
    }
}
