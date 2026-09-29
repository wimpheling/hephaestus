use super::{
    GatewayAuditDisposition, MAX_UI_REQUEST_BODY_BYTES, StaticRequestHeaders, UiContentError,
    UiContentState, UiGatewayDispatchAuthority, UiHttpRequest, error_response,
    gateway_audit_disposition, gateway_response, map_serving_error, parse_and_resolve_host,
    parse_child_cookie, redirect_response, request_authority, require_unsafe_origin,
    sanitize_guest_request_headers, serve_static,
};
use crate::ui_audit::{correlation_id, reason_for_content_error, verified_context};
use axum::{Router, routing::any};
use axum::{
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{Request, StatusCode},
    response::IntoResponse,
    response::Response,
};
use identity_domain::RequestId;
use release_service::ui_browser_serving::UiBrowserHttpRequest;
use release_service::ui_browser_serving::UiServingProjection;
use std::{net::SocketAddr, sync::Arc};

pub(super) fn router(state: Arc<UiContentState>) -> Router {
    Router::new()
        .fallback(any(content_request))
        .with_state(state)
}

pub(super) async fn content_request(
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
