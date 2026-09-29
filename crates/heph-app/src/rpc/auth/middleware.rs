use super::{HANDOFF_AUDIENCE, ISSUER, MediatorAuthenticationState, UiHandoffAuditMarker};
use crate::rpc::auth::claims::{MediatorAuthMode, auth_mode};
use crate::rpc::auth::state::SessionAuthenticationError;
use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::Next,
    response::Response,
};
use release_service::UiRequestAuditReason;

/// Axum middleware that validates a mediator assertion for the exact request
/// path and installs the resulting identity in request extensions.
///
/// The two bootstrap procedures validate their request-body identity later in
/// their typed RPC handlers. Revoke authenticates the signed session claims but
/// deliberately skips the active-row check so an expired session can log out.
/// Every other Connect RPC requires an active durable browser session.
pub async fn mediator_identity_middleware(
    State(state): State<MediatorAuthenticationState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let mode = auth_mode(request.uri().path());
    let handoff_marker = (request.uri().path() == HANDOFF_AUDIENCE).then(UiHandoffAuditMarker::new);
    if let Some(marker) = &handoff_marker {
        request.extensions_mut().insert(marker.clone());
    }
    let session = match mode {
        MediatorAuthMode::Public | MediatorAuthMode::Bootstrap => {
            return next.run(request).await;
        }
        MediatorAuthMode::Signed => {
            let Ok(session) = state.authenticate_signed(request.headers(), request.uri().path())
            else {
                if let Some(marker) = &handoff_marker {
                    state
                        .audit_handoff_denial(marker, UiRequestAuditReason::Unauthenticated)
                        .await;
                }
                return auth_response(StatusCode::UNAUTHORIZED);
            };
            session
        }
        MediatorAuthMode::Active => match state
            .authenticate_active(request.headers(), request.uri().path())
            .await
        {
            Ok(session) => session,
            Err(SessionAuthenticationError::Unauthenticated) => {
                if let Some(marker) = &handoff_marker {
                    state
                        .audit_handoff_denial(marker, UiRequestAuditReason::Unauthenticated)
                        .await;
                }
                return auth_response(StatusCode::UNAUTHORIZED);
            }
            Err(SessionAuthenticationError::Unavailable) => {
                if let Some(marker) = &handoff_marker {
                    state
                        .audit_handoff_denial(marker, UiRequestAuditReason::Unavailable)
                        .await;
                }
                return auth_response(StatusCode::SERVICE_UNAVAILABLE);
            }
        },
    };
    if let Some(marker) = &handoff_marker {
        marker.set_actor(session.user_id);
    }
    request.extensions_mut().insert(session);
    request
        .extensions_mut()
        .insert(identity_domain::AuthenticatedIdentity::new(
            session.user_id,
            ISSUER,
            session.user_id.to_string(),
            serde_json::json!({"mediator": "phoenix", "assertion_id": session.assertion_id}),
            identity_domain::RequestId::from_uuid(session.assertion_id),
        ));
    let response = next.run(request).await;
    if let Some(marker) = &handoff_marker {
        if !marker.handler_reached() {
            let reason = if response.status().is_server_error() {
                UiRequestAuditReason::Unavailable
            } else {
                UiRequestAuditReason::InvalidInput
            };
            state.audit_handoff_denial(marker, reason).await;
        }
    }
    response
}

fn auth_response(status: StatusCode) -> Response {
    let mut response = Response::new(Body::empty());
    *response.status_mut() = status;
    response
}
