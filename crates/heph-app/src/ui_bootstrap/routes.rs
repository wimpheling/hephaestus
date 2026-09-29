use axum::{
    Router,
    body::Body,
    extract::{ConnectInfo, State},
    http::{HeaderValue, Request, StatusCode, header},
    response::{IntoResponse, Response},
    routing::get,
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use release_service::ui_browser_host::{UI_BOOTSTRAP_PATH, UI_CHILD_COOKIE};
use serde::Serialize;
use std::{net::SocketAddr, sync::Arc};
use uuid::Uuid;

use super::{
    config::UiBootstrapState,
    helpers::{
        add_no_store, bootstrap_html, child_max_age, error_response, exact_origin_matches,
        exchange, exchange_error_response, insert_header, parse_theme, request_authority,
        resolve_host,
    },
};
use crate::ui_audit::{correlation_id, reason_for_bootstrap_error};

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

#[derive(Debug, Serialize)]
struct BootstrapResponse {
    route: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    theme_origin: Option<String>,
}
