//! The narrow UI-origin bootstrap HTTP boundary.
//!
//! This module is intentionally limited to `/_heph/bootstrap`.  It resolves
//! the exact generation host before inspecting the handoff, exchanges the
//! one-time bearer through the release service port, and returns only a safe
//! route. Static files and managed/API forwarding belong to later handlers.
//!
//! Intended home: `crates/hephaestus-app/src/ui_bootstrap.rs`, after the host
//! and read-port candidates are integrated into `release-service`.

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use identity_domain::RequestId;
use release_domain::ui_browser::{UiBrowserHandoffSecret, UiBrowserSessionSecret};
use release_service::ui_browser_host::{
    UI_BOOTSTRAP_PATH, UI_CHILD_COOKIE, UI_HANDOFF_FRAGMENT_LENGTH, UiGenerationHost, UiNamespace,
    UiPublicPort, is_handoff_fragment,
};
use release_service::ui_browser_serving::{
    ActiveUiGenerationHost, UiGenerationHostResolver, UiHostLookupError,
};
use release_service::{ExchangeUiBrowserHandoff, UiBrowserHandoffError, UiBrowserSessionStore};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc, time::Duration};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::{
    ui_audit::{UiAuditRecorder, correlation_id, reason_for_bootstrap_error},
    ui_origin_config::{UiOriginConfig, UiOriginConfigError},
};

/// The raw handoff is exactly 43 ASCII bytes. The extra allowance prevents a
/// transport implementation from buffering an unbounded malformed body.
pub const MAX_HANDOFF_BODY_BYTES: usize = 128;
/// Bootstrap must not leave a request waiting behind a stuck app-pool call.
pub const DEFAULT_BOOTSTRAP_DEADLINE: Duration = Duration::from_secs(5);
/// The listener has a separate bounded permit pool from the gateway worker.
pub const DEFAULT_BOOTSTRAP_CONCURRENCY: usize = 64;

/// Configuration that is safe to use in response headers and bootstrap HTML.
#[derive(Debug, Clone)]
pub struct UiBootstrapConfig {
    origin: UiOriginConfig,
}

impl UiBootstrapConfig {
    /// Validates exact HTTPS origins and requires the UI namespace to be a
    /// strict subdomain of the configured platform host. This keeps the
    /// host-only cookie same-site without a public-suffix implementation.
    pub fn new(
        namespace: UiNamespace,
        public_port: UiPublicPort,
        platform_origin: impl Into<String>,
    ) -> Result<Self, UiBootstrapConfigError> {
        Ok(Self {
            origin: UiOriginConfig::new(namespace, public_port, platform_origin)
                .map_err(UiBootstrapConfigError::from)?,
        })
    }

    /// Returns the configured namespace.
    #[must_use]
    pub const fn namespace(&self) -> &UiNamespace {
        self.origin.namespace()
    }

    /// Returns the configured HTTPS public port.
    #[must_use]
    pub const fn public_port(&self) -> UiPublicPort {
        self.origin.public_port()
    }

    /// Returns the canonical platform HTTPS origin.
    #[must_use]
    pub fn platform_origin(&self) -> &str {
        self.origin.platform_origin()
    }
}

/// Configuration errors are kept separate from HTTP errors so startup can
/// fail closed instead of constructing a handler with unsafe origin policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum UiBootstrapConfigError {
    /// Platform frame ancestors must be one exact HTTPS origin.
    #[error("platform origin must be an exact HTTPS origin")]
    InvalidPlatformOrigin,
    /// Cookie same-site policy requires a strict platform-host subdomain.
    #[error("UI namespace must be a strict subdomain of the platform host")]
    NamespaceNotPlatformSubdomain,
}

impl From<UiOriginConfigError> for UiBootstrapConfigError {
    fn from(error: UiOriginConfigError) -> Self {
        match error {
            UiOriginConfigError::InvalidPlatformOrigin => Self::InvalidPlatformOrigin,
            UiOriginConfigError::NamespaceNotPlatformSubdomain => {
                Self::NamespaceNotPlatformSubdomain
            }
        }
    }
}

/// Dependencies for the bootstrap route. The HTTP layer never receives a
/// database pool and never accepts actor, parent, organization, or generation
/// identity from the browser body.
pub struct UiBootstrapState {
    host_resolver: Arc<dyn UiGenerationHostResolver>,
    sessions: Arc<dyn UiBrowserSessionStore>,
    config: UiBootstrapConfig,
    permits: Arc<Semaphore>,
    deadline: Duration,
    audit: UiAuditRecorder,
}

impl UiBootstrapState {
    /// Constructs a bounded handler state for the private loopback listener.
    #[must_use]
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        sessions: Arc<dyn UiBrowserSessionStore>,
        config: UiBootstrapConfig,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Self {
        Self {
            host_resolver,
            sessions,
            config,
            permits: Arc::new(Semaphore::new(DEFAULT_BOOTSTRAP_CONCURRENCY)),
            deadline: DEFAULT_BOOTSTRAP_DEADLINE,
            audit: UiAuditRecorder::new(audit_sink),
        }
    }

    /// Replaces the defaults for a focused listener test or an app-specific
    /// operational limit. A zero limit or zero deadline is rejected.
    #[allow(dead_code)] // Reserved for the daemon's listener composition layer.
    #[must_use]
    pub fn with_limits(mut self, concurrency: usize, deadline: Duration) -> Option<Self> {
        if concurrency == 0 || deadline.is_zero() {
            return None;
        }
        self.permits = Arc::new(Semaphore::new(concurrency));
        self.deadline = deadline;
        Some(self)
    }
}

/// Registers only the exact bootstrap paths. Content routes are deliberately
/// absent until their static/gateway serving boundaries are integrated.
pub fn router(state: Arc<UiBootstrapState>) -> Router {
    Router::new()
        .route(UI_BOOTSTRAP_PATH, get(get_bootstrap).post(post_bootstrap))
        .with_state(state)
}

// Keep the ordered authority, host, and response-audit phases together so a
// denial cannot accidentally bypass the same correlation path.
#[allow(clippy::too_many_lines)]
async fn get_bootstrap(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiBootstrapState>>,
    request: Request<Body>,
) -> Response {
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unauthorized,
                error_response(StatusCode::FORBIDDEN, "bootstrap_forbidden"),
            )
            .await;
    }
    let Ok(_permit) = state.permits.clone().try_acquire_owned() else {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unavailable,
                error_response(StatusCode::TOO_MANY_REQUESTS, "bootstrap_busy"),
            )
            .await;
    };
    let authority = match request_authority(&request) {
        Ok(authority) => authority,
        Err(status) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::InvalidInput,
                    error_response(status, "bootstrap_unavailable"),
                )
                .await;
        }
    };
    let _host = match tokio::time::timeout(state.deadline, resolve_host(&state, authority)).await {
        Ok(Ok(Some(host))) => host,
        Ok(Ok(None)) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::NotFound,
                    error_response(StatusCode::NOT_FOUND, "bootstrap_unavailable"),
                )
                .await;
        }
        Ok(Err(status)) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::InvalidInput,
                    error_response(status, "bootstrap_unavailable"),
                )
                .await;
        }
        Err(_) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::Unavailable,
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable"),
                )
                .await;
        }
    };
    let nonce = URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes());
    let html = bootstrap_html(&nonce, state.config.platform_origin());
    let mut response = Response::new(Body::from(html));
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    add_no_store(headers);
    insert_header(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    insert_header(
        headers,
        header::CONTENT_SECURITY_POLICY,
        &format!(
            "default-src 'none'; script-src 'nonce-{nonce}'; connect-src 'self'; frame-ancestors {}; base-uri 'none'; form-action 'none'",
            state.config.platform_origin()
        ),
    );
    state
        .audit
        .allowed(
            request_id,
            release_service::UiRequestAuditSurface::Bootstrap,
            release_service::UiRequestAuditContext::anonymous(),
            release_service::UiRequestAuditOutcome::Succeeded,
            release_service::UiRequestAuditReason::None,
            response,
        )
        .await
}

// Keep the ordered authority, exchange, and response-audit phases together so
// every terminal result retains the same correlation and timeout semantics.
#[allow(clippy::too_many_lines)]
async fn post_bootstrap(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<Arc<UiBootstrapState>>,
    request: Request<Body>,
) -> Response {
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unauthorized,
                error_response(StatusCode::FORBIDDEN, "bootstrap_forbidden"),
            )
            .await;
    }
    let Ok(_permit) = state.permits.clone().try_acquire_owned() else {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unavailable,
                error_response(StatusCode::TOO_MANY_REQUESTS, "bootstrap_busy"),
            )
            .await;
    };
    let authority = match request_authority(&request) {
        Ok(authority) => authority,
        Err(status) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::InvalidInput,
                    error_response(status, "bootstrap_unavailable"),
                )
                .await;
        }
    };
    let host = match tokio::time::timeout(state.deadline, resolve_host(&state, authority)).await {
        Ok(Ok(Some(host))) => host,
        Ok(Ok(None)) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::NotFound,
                    error_response(StatusCode::NOT_FOUND, "bootstrap_unavailable"),
                )
                .await;
        }
        Ok(Err(status)) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::InvalidInput,
                    error_response(status, "bootstrap_unavailable"),
                )
                .await;
        }
        Err(_) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditReason::Unavailable,
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable"),
                )
                .await;
        }
    };
    if !exact_origin_matches(&request, &host, &state.config) {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unauthorized,
                error_response(StatusCode::FORBIDDEN, "bootstrap_forbidden"),
            )
            .await;
    }
    let Ok(theme) = parse_theme(request.uri().query()) else {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::InvalidInput,
                error_response(StatusCode::BAD_REQUEST, "bootstrap_invalid_request"),
            )
            .await;
    };
    let exchange =
        tokio::time::timeout(state.deadline, exchange(&state, request_id, request, host)).await;
    let created = match exchange {
        Ok(Ok(created)) => created,
        Ok(Err(error)) => {
            return state
                .audit
                .denial(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    reason_for_bootstrap_error(error),
                    exchange_error_response(error),
                )
                .await;
        }
        Err(_) => {
            return state
                .audit
                .undetermined(
                    request_id,
                    release_service::UiRequestAuditSurface::Bootstrap,
                    release_service::UiRequestAuditContext::anonymous(),
                    error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable"),
                )
                .await;
        }
    };
    let Some(max_age) = child_max_age(created.expires_at) else {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Bootstrap,
                release_service::UiRequestAuditReason::Unavailable,
                error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable"),
            )
            .await;
    };
    let cookie = format!(
        "{UI_CHILD_COOKIE}={}; Path=/; Max-Age={max_age}; Secure; HttpOnly; SameSite=Strict",
        created.child_cookie_value
    );
    let route = format!("/{}{}", created.context.route, theme.query_suffix());
    let body = BootstrapResponse {
        route,
        theme: theme.as_str().map(str::to_owned),
        // The client may carry this only into the final route after checking
        // it against the exact origin embedded in the trusted bootstrap HTML.
        theme_origin: Some(state.config.platform_origin().to_owned()),
    };
    let mut response = axum::Json(body).into_response();
    *response.status_mut() = StatusCode::OK;
    let headers = response.headers_mut();
    add_no_store(headers);
    insert_header(headers, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    insert_header(headers, header::SET_COOKIE, &cookie);
    state
        .audit
        .allowed(
            request_id,
            release_service::UiRequestAuditSurface::Bootstrap,
            release_service::UiRequestAuditContext::verified(
                created.context.actor_id,
                created.context.organization_id,
                created.context.installation_id,
                created.context.generation_id,
                Some(created.context.session_id),
                None,
            ),
            release_service::UiRequestAuditOutcome::Succeeded,
            release_service::UiRequestAuditReason::None,
            response,
        )
        .await
}

async fn resolve_host(
    state: &UiBootstrapState,
    authority: String,
) -> Result<Option<UiGenerationHost>, StatusCode> {
    let host = UiGenerationHost::parse(
        &authority,
        state.config.namespace(),
        state.config.public_port(),
    )
    .map_err(|_| StatusCode::NOT_FOUND)?;
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
    {
        Ok(Some(ActiveUiGenerationHost { generation_id }))
            if generation_id == host.generation_id() =>
        {
            Ok(Some(host))
        }
        Ok(Some(_) | None) => Ok(None),
        Err(UiHostLookupError::Unavailable) => Err(StatusCode::SERVICE_UNAVAILABLE),
    }
}

fn request_authority(request: &Request<Body>) -> Result<String, StatusCode> {
    if request.uri().scheme().is_some()
        || request.uri().authority().is_some()
        || request.headers().get_all(header::HOST).iter().count() != 1
    {
        return Err(StatusCode::BAD_REQUEST);
    }
    request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .ok_or(StatusCode::BAD_REQUEST)
}

async fn exchange(
    state: &UiBootstrapState,
    request_id: RequestId,
    request: Request<Body>,
    host: UiGenerationHost,
) -> Result<ExchangeCreated, UiBrowserHandoffError> {
    let body = to_bytes(request.into_body(), MAX_HANDOFF_BODY_BYTES)
        .await
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let text = std::str::from_utf8(&body).map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    if !is_handoff_fragment(text) || text.len() != UI_HANDOFF_FRAGMENT_LENGTH {
        return Err(UiBrowserHandoffError::InvalidOrExpired);
    }
    let bytes = URL_SAFE_NO_PAD
        .decode(text)
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let bytes: [u8; 32] = bytes
        .try_into()
        .map_err(|_| UiBrowserHandoffError::InvalidOrExpired)?;
    let handoff_secret = UiBrowserHandoffSecret::from_bytes(bytes);
    let child_secret = UiBrowserSessionSecret::random();
    let child_cookie_value = URL_SAFE_NO_PAD.encode(child_secret.as_bytes());
    let created = state
        .sessions
        .exchange_ui_browser_handoff(ExchangeUiBrowserHandoff {
            request_id,
            handoff_secret,
            expected_generation_id: host.generation_id(),
            child_secret,
        })
        .await?;
    let expires_at = created.context.expires_at;
    Ok(ExchangeCreated {
        context: created.context,
        expires_at,
        child_cookie_value,
    })
}

/// A private transport-only value. The raw child secret is consumed once to
/// build `Set-Cookie` and never crosses the JSON response boundary.
struct ExchangeCreated {
    context: release_service::UiBrowserSessionContext,
    expires_at: OffsetDateTime,
    child_cookie_value: String,
}

#[derive(Debug, Serialize)]
struct BootstrapResponse {
    route: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme_origin: Option<String>,
}

fn exchange_error_response(error: UiBrowserHandoffError) -> Response {
    match error {
        UiBrowserHandoffError::PermissionDenied
        | UiBrowserHandoffError::InvalidRoute
        | UiBrowserHandoffError::InvalidOrExpired => {
            error_response(StatusCode::UNAUTHORIZED, "bootstrap_unauthorized")
        }
        UiBrowserHandoffError::Unavailable => {
            error_response(StatusCode::SERVICE_UNAVAILABLE, "bootstrap_unavailable")
        }
    }
}

fn error_response(status: StatusCode, code: &'static str) -> Response {
    let mut response = (status, axum::Json(ErrorResponse { error: code })).into_response();
    add_no_store(response.headers_mut());
    insert_header(
        response.headers_mut(),
        header::X_CONTENT_TYPE_OPTIONS,
        "nosniff",
    );
    response
}

#[derive(Debug, Serialize)]
struct ErrorResponse {
    error: &'static str,
}

fn exact_origin_matches(
    request: &Request<Body>,
    host: &UiGenerationHost,
    config: &UiBootstrapConfig,
) -> bool {
    if request.headers().get_all(header::ORIGIN).iter().count() != 1 {
        return false;
    }
    let Some(origin) = request.headers().get(header::ORIGIN) else {
        return false;
    };
    let Ok(origin) = origin.to_str() else {
        return false;
    };
    origin
        == format!(
            "https://{}",
            host.authority(config.namespace(), config.public_port())
        )
}

fn child_max_age(expires_at: OffsetDateTime) -> Option<i64> {
    let seconds = (expires_at - OffsetDateTime::now_utc()).whole_seconds();
    (1..=12 * 60 * 60).contains(&seconds).then_some(seconds)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Theme {
    Light,
    Dark,
}

impl Theme {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    const fn query_suffix(self) -> &'static str {
        match self {
            Self::Light => "?heph_theme=light",
            Self::Dark => "?heph_theme=dark",
        }
    }
}

fn parse_theme(query: Option<&str>) -> Result<ThemeOption, ()> {
    let mut theme = None;
    for pair in query
        .unwrap_or_default()
        .split('&')
        .filter(|pair| !pair.is_empty())
    {
        let Some((key, value)) = pair.split_once('=') else {
            if pair == "heph_theme" {
                return Err(());
            }
            continue;
        };
        if key != "heph_theme" {
            continue;
        }
        if theme.is_some() {
            return Err(());
        }
        theme = Some(match value {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => return Err(()),
        });
    }
    Ok(ThemeOption(theme))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThemeOption(Option<Theme>);

impl ThemeOption {
    fn as_str(self) -> Option<&'static str> {
        self.0.map(Theme::as_str)
    }

    fn query_suffix(self) -> &'static str {
        self.0.map_or("", Theme::query_suffix)
    }
}

fn add_no_store(headers: &mut HeaderMap) {
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
}

fn insert_header(headers: &mut HeaderMap, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

fn bootstrap_html(nonce: &str, platform_origin: &str) -> String {
    let platform_meta = format!(
        "<meta name=\"heph-platform-origin\" content=\"{}\">",
        escape_attribute(platform_origin)
    );
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"referrer\" content=\"no-referrer\">{platform_meta}</head><body><p>Opening Heph UI…</p><script nonce=\"{nonce}\">{BOOTSTRAP_SCRIPT}</script></body></html>"
    )
}

const BOOTSTRAP_SCRIPT: &str = include_str!("ui_bootstrap.js");

fn escape_attribute(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{body::to_bytes, http::Request};
    use release_domain::{
        UiInstallationGenerationId, UiInstallationId,
        ui_browser::{UiBrowserRoute, UiBrowserSessionId},
    };
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    #[test]
    fn theme_is_allowlisted_and_not_reflected() {
        assert_eq!(
            parse_theme(Some("heph_theme=light")),
            Ok(ThemeOption(Some(Theme::Light)))
        );
        assert!(parse_theme(Some("heph_theme=anything")).is_err());
        assert!(parse_theme(Some("heph_theme=light&heph_theme=dark")).is_err());
        assert_eq!(
            parse_theme(Some("redirect=https://evil")),
            Ok(ThemeOption(None))
        );
    }

    #[test]
    fn bootstrap_script_clears_fragment_before_post_and_requires_host_relative_route() {
        let html = bootstrap_html("nonce", "https://app.example");
        assert!(html.contains("history.replaceState(null,\"\",location.pathname+initialSearch)"));
        assert!(html.contains("fetch(\"/_heph/bootstrap\"+bootstrapQuery"));
        assert!(html.contains("heph_theme_origin"));
        assert!(html.contains("body:fragment"));
        assert!(html.contains("location.replace(destination.pathname"));
        assert!(!html.contains("parent_session_id"));
        assert!(html.contains("heph-platform-origin"));
    }

    #[test]
    fn cookie_policy_has_no_domain_or_platform_cookie() {
        let value = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let cookie = format!(
            "{UI_CHILD_COOKIE}={value}; Path=/; Max-Age=60; Secure; HttpOnly; SameSite=Strict"
        );
        assert!(cookie.starts_with("__Host-hephaestus_ui="));
        assert!(!cookie.contains("Domain="));
        assert!(!cookie.contains("platform"));
    }

    #[test]
    fn csp_uses_exact_platform_frame_ancestor() {
        let html = bootstrap_html("nonce", "https://app.example");
        assert!(html.contains("nonce=\"nonce\""));
        assert!(html.contains("heph-platform-origin"));
        assert!(!html.contains("g-*"));
    }

    #[test]
    fn origin_configuration_rejects_paths_and_sibling_namespaces() {
        let namespace = UiNamespace::parse("ui.app.example").expect("test namespace");
        let port = UiPublicPort::https_default();
        assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example").is_ok());
        assert!(
            UiBootstrapConfig::new(namespace.clone(), port, "https://app.example/path").is_err()
        );
        assert!(UiBootstrapConfig::new(namespace, port, "https://other.example").is_err());
    }

    #[test]
    fn exact_origin_rejects_ambiguous_authorities() {
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        let port = UiPublicPort::https_default();
        assert!(UiBootstrapConfig::new(namespace.clone(), port, "https://app.example").is_ok());
        assert!(
            UiBootstrapConfig::new(namespace.clone(), port, "https://app.example/path").is_err()
        );
        assert!(
            UiBootstrapConfig::new(namespace.clone(), port, "https://user@app.example").is_err()
        );
        assert!(UiBootstrapConfig::new(namespace, port, "https://app.example:443:444").is_err());
    }

    #[derive(Clone)]
    struct FakeHostResolver {
        generation_id: UiInstallationGenerationId,
        calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl UiGenerationHostResolver for FakeHostResolver {
        async fn resolve_active_generation_host(
            &self,
            host: UiGenerationHost,
        ) -> Result<Option<ActiveUiGenerationHost>, UiHostLookupError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(
                (host.generation_id() == self.generation_id).then_some(ActiveUiGenerationHost {
                    generation_id: self.generation_id,
                }),
            )
        }
    }

    struct FakeSessions {
        exchanges: Arc<AtomicUsize>,
        reject: bool,
    }

    #[async_trait]
    impl UiBrowserSessionStore for FakeSessions {
        async fn create_ui_browser_handoff(
            &self,
            _command: release_service::CreateUiBrowserHandoff,
        ) -> Result<release_service::CreatedUiBrowserHandoff, UiBrowserHandoffError> {
            panic!("bootstrap test must not issue handoffs")
        }

        async fn exchange_ui_browser_handoff(
            &self,
            command: release_service::ExchangeUiBrowserHandoff,
        ) -> Result<release_service::CreatedUiBrowserSession, UiBrowserHandoffError> {
            self.exchanges.fetch_add(1, Ordering::SeqCst);
            if self.reject {
                return Err(UiBrowserHandoffError::InvalidOrExpired);
            }
            Ok(release_service::CreatedUiBrowserSession {
                context: release_service::UiBrowserSessionContext {
                    session_id: UiBrowserSessionId::from_uuid(Uuid::new_v4()),
                    parent_session_id: identity_domain::BrowserSessionId::from_uuid(Uuid::new_v4()),
                    actor_id: identity_domain::UserId::from_uuid(Uuid::new_v4()),
                    organization_id: forge_domain::OrganizationId::from_uuid(Uuid::new_v4()),
                    installation_id: UiInstallationId::from_uuid(Uuid::new_v4()),
                    generation_id: command.expected_generation_id,
                    route: UiBrowserRoute::parse("schema-ui").expect("route"),
                    expires_at: OffsetDateTime::now_utc() + time::Duration::hours(1),
                },
            })
        }

        async fn authenticate_ui_browser_session(
            &self,
            _command: release_service::AuthenticateUiBrowserSession,
        ) -> Result<release_service::UiBrowserSessionContext, release_service::UiBrowserSessionError>
        {
            panic!("bootstrap test must not authenticate content")
        }
    }

    fn bootstrap_state(
        generation_id: UiInstallationGenerationId,
        host_calls: Arc<AtomicUsize>,
        exchange_calls: Arc<AtomicUsize>,
    ) -> (Arc<UiBootstrapState>, String) {
        bootstrap_state_with_rejection(generation_id, host_calls, exchange_calls, false)
    }

    fn bootstrap_state_with_rejection(
        generation_id: UiInstallationGenerationId,
        host_calls: Arc<AtomicUsize>,
        exchange_calls: Arc<AtomicUsize>,
        reject: bool,
    ) -> (Arc<UiBootstrapState>, String) {
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        let port = UiPublicPort::https_default();
        let host = UiGenerationHost::from_generation_id(generation_id);
        let authority = host.authority(&namespace, port);
        let state = UiBootstrapState::new(
            Arc::new(FakeHostResolver {
                generation_id,
                calls: host_calls,
            }),
            Arc::new(FakeSessions {
                exchanges: exchange_calls,
                reject,
            }),
            UiBootstrapConfig::new(namespace, port, "https://app.example").expect("config"),
            Arc::new(crate::ui_audit::NoopAuditSink),
        );
        (Arc::new(state), authority)
    }

    #[tokio::test]
    async fn bootstrap_router_appends_correlation_before_exchange_denial() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let host_calls = Arc::new(AtomicUsize::new(0));
        let exchange_calls = Arc::new(AtomicUsize::new(0));
        let sink = Arc::new(crate::ui_audit::CapturingAuditSink::default());
        let namespace = UiNamespace::parse("ui.app.example").expect("namespace");
        let port = UiPublicPort::https_default();
        let host = UiGenerationHost::from_generation_id(generation_id);
        let authority = host.authority(&namespace, port);
        let state = Arc::new(UiBootstrapState::new(
            Arc::new(FakeHostResolver {
                generation_id,
                calls: host_calls,
            }),
            Arc::new(FakeSessions {
                exchanges: exchange_calls,
                reject: true,
            }),
            UiBootstrapConfig::new(namespace, port, "https://app.example").expect("config"),
            Arc::clone(&sink) as Arc<dyn release_service::UiRequestAuditSink>,
        ));
        let mut request = Request::builder()
            .method("POST")
            .uri(UI_BOOTSTRAP_PATH)
            .header(header::HOST, &authority)
            .header(header::ORIGIN, format!("https://{authority}"))
            .body(Body::from(URL_SAFE_NO_PAD.encode([7_u8; 32])))
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let events = sink.events.lock().expect("events");
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0].surface(),
            release_service::UiRequestAuditSurface::Bootstrap
        );
        assert_eq!(
            events[0].decision(),
            release_service::UiRequestAuditDecision::Denied
        );
        assert!(events[0].context().actor_id().is_none());
        drop(events);
    }

    #[tokio::test]
    async fn bootstrap_router_exchanges_valid_fragment_and_returns_safe_cookie_json() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let host_calls = Arc::new(AtomicUsize::new(0));
        let exchange_calls = Arc::new(AtomicUsize::new(0));
        let (state, authority) = bootstrap_state(
            generation_id,
            Arc::clone(&host_calls),
            Arc::clone(&exchange_calls),
        );
        let fragment = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let mut request = Request::builder()
            .method("POST")
            .uri(UI_BOOTSTRAP_PATH)
            .header(header::HOST, &authority)
            .header(header::ORIGIN, format!("https://{authority}"))
            .body(Body::from(fragment.clone()))
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .expect("child cookie");
        assert!(cookie.starts_with("__Host-hephaestus_ui="));
        assert!(cookie.contains("Secure"));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));
        assert!(!cookie.contains("Domain="));
        let body = to_bytes(response.into_body(), 4096).await.expect("body");
        let body = String::from_utf8(body.to_vec()).expect("JSON");
        assert!(body.contains("/schema-ui"));
        assert!(!body.contains(&fragment));
        assert_eq!(host_calls.load(Ordering::SeqCst), 1);
        assert_eq!(exchange_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn bootstrap_router_rejects_duplicate_host_origin_and_non_loopback_before_exchange() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let host_calls = Arc::new(AtomicUsize::new(0));
        let exchange_calls = Arc::new(AtomicUsize::new(0));
        let (state, authority) = bootstrap_state(
            generation_id,
            Arc::clone(&host_calls),
            Arc::clone(&exchange_calls),
        );
        let fragment = URL_SAFE_NO_PAD.encode([7_u8; 32]);
        let mut duplicate = Request::builder()
            .method("POST")
            .uri(UI_BOOTSTRAP_PATH)
            .header(header::HOST, &authority)
            .header(header::HOST, &authority)
            .header(header::ORIGIN, format!("https://{authority}"))
            .body(Body::from(fragment.clone()))
            .expect("request");
        duplicate
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        let response = router(Arc::clone(&state))
            .oneshot(duplicate)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(exchange_calls.load(Ordering::SeqCst), 0);

        let mut absolute = Request::builder()
            .method("POST")
            .uri(format!("https://{authority}{UI_BOOTSTRAP_PATH}"))
            .header(header::HOST, &authority)
            .header(header::ORIGIN, format!("https://{authority}"))
            .body(Body::from(fragment.clone()))
            .expect("absolute-form request");
        absolute
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        let response = router(Arc::clone(&state))
            .oneshot(absolute)
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        let mut foreign = Request::builder()
            .method("POST")
            .uri(UI_BOOTSTRAP_PATH)
            .header(header::HOST, &authority)
            .header(header::ORIGIN, "https://other.example")
            .body(Body::from(fragment))
            .expect("request");
        foreign
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([192, 0, 2, 1], 4321))));
        let response = router(state).oneshot(foreign).await.expect("response");
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(exchange_calls.load(Ordering::SeqCst), 0);
        assert_eq!(host_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn bootstrap_router_rejects_expired_exchange_without_cookie_or_safe_route() {
        let generation_id = UiInstallationGenerationId::from_uuid(Uuid::new_v4());
        let host_calls = Arc::new(AtomicUsize::new(0));
        let exchange_calls = Arc::new(AtomicUsize::new(0));
        let (state, authority) = bootstrap_state_with_rejection(
            generation_id,
            host_calls,
            Arc::clone(&exchange_calls),
            true,
        );
        let mut request = Request::builder()
            .method("POST")
            .uri(UI_BOOTSTRAP_PATH)
            .header(header::HOST, &authority)
            .header(header::ORIGIN, format!("https://{authority}"))
            .body(Body::from(URL_SAFE_NO_PAD.encode([8_u8; 32])))
            .expect("request");
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 4321))));
        let response = router(state).oneshot(request).await.expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert!(response.headers().get(header::SET_COOKIE).is_none());
        assert_eq!(exchange_calls.load(Ordering::SeqCst), 1);
    }
}
