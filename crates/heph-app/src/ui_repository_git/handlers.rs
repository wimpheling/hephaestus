use axum::{
    body::Body,
    http::{Request, StatusCode, header},
    response::Response,
};
use forge_domain::RepositoryId;
use git_http::{AuthenticatedHumanGitRequest, execute_authenticated_human};
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_service::{
    ActiveUiGenerationHost, UiGitAuthorizationError, UiHostLookupError, UiRepositoryGitOperation,
    ui_browser_host::UiGenerationHost,
};
use std::{net::SocketAddr, sync::Arc};

use super::{
    common::{GitRoute, MAX_HEADER_BYTES, MAX_QUERY_BYTES, UiRepositoryGitState},
    validation::{actor_header, parse_child_cookie, request_authority, require_same_origin},
};
use crate::ui_audit::correlation_id;

// Keep the security gates visibly ordered in one adapter: loopback, bounded
// syntax, canonical host, origin, live authority, then Git execution.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub(super) async fn handle(
    peer: SocketAddr,
    state: Arc<UiRepositoryGitState>,
    route: GitRoute,
    mut request: Request<Body>,
) -> Response<Body> {
    let request_id = correlation_id(request.extensions());
    if !peer.ip().is_loopback() {
        return state
            .audit
            .denial(
                request_id,
                release_service::UiRequestAuditSurface::Content,
                release_service::UiRequestAuditReason::Unauthorized,
                super::validation::error_response(StatusCode::FORBIDDEN, "ui_forbidden"),
            )
            .await;
    }
    let header_bytes = request
        .headers()
        .iter()
        .map(|(name, value)| name.as_str().len().saturating_add(value.as_bytes().len()))
        .sum::<usize>();
    if header_bytes > MAX_HEADER_BYTES {
        return reject(
            &state,
            request_id,
            StatusCode::REQUEST_HEADER_FIELDS_TOO_LARGE,
            "ui_git_headers",
        )
        .await;
    }
    let repository_id = match route.repository.parse::<uuid::Uuid>() {
        Ok(uuid) if uuid.to_string() == route.repository => RepositoryId::from_uuid(uuid),
        Ok(_) | Err(_) => {
            return reject(
                &state,
                request_id,
                StatusCode::BAD_REQUEST,
                "ui_git_repository",
            )
            .await;
        }
    };
    if request
        .uri()
        .query()
        .is_some_and(|value| value.len() > MAX_QUERY_BYTES)
        || (route.endpoint.backend_endpoint() != "info/refs" && request.uri().query().is_some())
    {
        return reject(&state, request_id, StatusCode::URI_TOO_LONG, "ui_git_query").await;
    }
    let authority = match request_authority(&request) {
        Ok(authority) => authority,
        Err(response) => return reject(&state, request_id, response, "ui_git_authority").await,
    };
    let Ok(host) = UiGenerationHost::parse(&authority, &state.namespace, state.public_port) else {
        return reject(
            &state,
            request_id,
            StatusCode::NOT_FOUND,
            "ui_git_not_found",
        )
        .await;
    };
    match state
        .host_resolver
        .resolve_active_generation_host(host)
        .await
    {
        Ok(Some(ActiveUiGenerationHost { generation_id }))
            if generation_id == host.generation_id() => {}
        Ok(Some(_) | None) => {
            return reject(
                &state,
                request_id,
                StatusCode::NOT_FOUND,
                "ui_git_not_found",
            )
            .await;
        }
        Err(UiHostLookupError::Unavailable) => {
            return reject(
                &state,
                request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "ui_unavailable",
            )
            .await;
        }
    }
    if let Err(status) = require_same_origin(
        &request,
        host,
        &state.namespace,
        state.public_port,
        route.endpoint.backend_endpoint() != "info/refs",
    ) {
        return reject(&state, request_id, status, "ui_git_origin").await;
    }
    if let Some(length) = request.headers().get(header::CONTENT_LENGTH) {
        let Ok(length) = length
            .to_str()
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or(())
        else {
            return reject(&state, request_id, StatusCode::BAD_REQUEST, "ui_git_length").await;
        };
        if length > state.git.limits().max_request_bytes {
            return reject(
                &state,
                request_id,
                StatusCode::PAYLOAD_TOO_LARGE,
                "ui_git_body",
            )
            .await;
        }
    }
    let secret = match parse_child_cookie(&request) {
        Ok(secret) => secret,
        Err(status) => return reject(&state, request_id, status, "ui_git_cookie").await,
    };
    let authorization = match state
        .authority
        .authorize_repository_git(
            request_id,
            secret,
            host.generation_id(),
            repository_id,
            route.authority_operation,
        )
        .await
    {
        Ok(authorization)
            if authorization.repository_id == repository_id
                && match route.authority_operation {
                    UiRepositoryGitOperation::Read => !matches!(
                        authorization.access,
                        release_domain::ui::UiRepositoryGitAccess::None
                    ),
                    UiRepositoryGitOperation::Write => matches!(
                        authorization.access,
                        release_domain::ui::UiRepositoryGitAccess::ReadWrite
                    ),
                } =>
        {
            authorization
        }
        Ok(_) | Err(UiGitAuthorizationError::Unauthorized) => {
            return reject(
                &state,
                request_id,
                StatusCode::FORBIDDEN,
                "ui_git_forbidden",
            )
            .await;
        }
        Err(UiGitAuthorizationError::Unavailable) => {
            return reject(
                &state,
                request_id,
                StatusCode::SERVICE_UNAVAILABLE,
                "ui_unavailable",
            )
            .await;
        }
    };
    // The UI cookie and any browser bearer header end at this boundary.
    request.headers_mut().remove(header::COOKIE);
    request.headers_mut().remove(header::AUTHORIZATION);
    let identity = AuthenticatedIdentity::new(
        authorization.actor_id,
        "urn:hephaestus:ui-browser",
        format!("user:{}", authorization.actor_id),
        serde_json::Value::Object(serde_json::Map::new()),
        request_id,
    );
    let response = tokio::time::timeout(
        state.deadline,
        execute_authenticated_human(
            Arc::clone(&state.git),
            AuthenticatedHumanGitRequest {
                repository_id,
                endpoint: route.endpoint,
                request,
                identity,
            },
        ),
    )
    .await;
    match response {
        Ok(response) if response.status().is_success() => {
            let response = actor_header(response, authorization.actor_id, route.endpoint);
            state
                .audit
                .allowed(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    release_service::UiRequestAuditOutcome::Succeeded,
                    release_service::UiRequestAuditReason::None,
                    response,
                )
                .await
        }
        Ok(response) => {
            state
                .audit
                .failed(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    release_service::UiRequestAuditReason::UpstreamFailure,
                    response,
                )
                .await
        }
        Err(_) => {
            state
                .audit
                .undetermined(
                    request_id,
                    release_service::UiRequestAuditSurface::Content,
                    crate::ui_audit::verified_context(&authorization.context),
                    super::validation::error_response(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "ui_unavailable",
                    ),
                )
                .await
        }
    }
}

pub(super) async fn reject(
    state: &UiRepositoryGitState,
    request_id: RequestId,
    status: StatusCode,
    message: &'static str,
) -> Response<Body> {
    state
        .audit
        .denial(
            request_id,
            release_service::UiRequestAuditSurface::Content,
            if status == StatusCode::NOT_FOUND {
                release_service::UiRequestAuditReason::NotFound
            } else if status == StatusCode::SERVICE_UNAVAILABLE {
                release_service::UiRequestAuditReason::Unavailable
            } else {
                release_service::UiRequestAuditReason::InvalidInput
            },
            super::validation::error_response(status, message),
        )
        .await
}
