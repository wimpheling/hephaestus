use super::{
    CreateUiBrowserHandoff, HANDOFF_AUDIENCE, ReleaseRpc, RequestContext, Response, ServiceRequest,
    ServiceResult, UiHandoffAuditMarker, UiRequestAuditContext, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface, UserId,
    append_ui_request_audit_bounded, generation_id, handoff_error, installation_id,
    into_connect_error, opaque, parse_handoff_secret, request, timestamp,
};
use crate::rpc::RpcError;
use identity_domain::{AuthenticatedIdentity, RequestId};
use release_domain::ui_browser::UiBrowserRoute;
use std::str::FromStr;
use uuid::Uuid;

/// Handles the non-replayable public handoff issuance boundary.
// Keep validation and audit-denial ordering in one boundary so every rejected
// field carries the same verified actor and transport correlation.
#[allow(clippy::too_many_lines)]
pub async fn handoff(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<
        '_,
        rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffRequest,
    >,
) -> ServiceResult<rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    if let Some(marker) = ctx.extensions().get::<UiHandoffAuditMarker>() {
        marker.mark_handler_reached();
    }
    let actor_hint = ctx
        .extensions()
        .get::<AuthenticatedIdentity>()
        .map(|identity| identity.user_id);
    let Ok(identity) = request::query_identity(&ctx, &service.authenticator, HANDOFF_AUDIENCE)
    else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            request.context.as_option(),
            actor_hint,
            RpcError::Unauthenticated,
            UiRequestAuditReason::Unauthenticated,
        )
        .await);
    };
    let Ok(verified) = request::verified_mediator_session(&ctx) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            request.context.as_option(),
            Some(identity.user_id),
            RpcError::Unauthenticated,
            UiRequestAuditReason::Unauthenticated,
        )
        .await);
    };
    let Some(parent_session_id) = verified.parent_session_id else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            request.context.as_option(),
            Some(identity.user_id),
            RpcError::Unauthenticated,
            UiRequestAuditReason::Unauthenticated,
        )
        .await);
    };
    let Some(context) = request.context.as_option() else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            None,
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    if !context.idempotency_key.is_empty() {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    }
    let Ok(request_id_text) = request::required_id(context.request_id.as_option()) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    let Ok(caller_request_id) = RequestId::from_str(&request_id_text) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    // The middleware marker is the one server-generated correlation carried
    // across transport, handler, and worker-store phases. The caller's
    // RequestContext ID is still parsed above as a required wire field, but
    // it is not allowed to split one request into two audit/store IDs.
    let request_id = ctx
        .extensions()
        .get::<UiHandoffAuditMarker>()
        .map_or(caller_request_id, UiHandoffAuditMarker::request_id);
    let Ok(secret) = parse_handoff_secret(&request.handoff_secret) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    let Ok(installation_id) = installation_id(request.installation_id.as_option()) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    let Ok(generation_id) = generation_id(request.generation_id.as_option()) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    let Ok(route) = UiBrowserRoute::parse(request.route) else {
        return Err(audit_handoff_denial(
            service,
            &budget,
            &ctx,
            Some(context),
            Some(identity.user_id),
            RpcError::InvalidArgument,
            UiRequestAuditReason::InvalidInput,
        )
        .await);
    };
    let created = request::run_with_budget(
        &budget,
        service
            .ui_browser
            .create_ui_browser_handoff(CreateUiBrowserHandoff {
                request_id,
                actor_id: identity.user_id,
                parent_session_id,
                installation_id,
                generation_id,
                route,
                secret,
            }),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(handoff_error)
    .map_err(into_connect_error)?;
    Response::ok(
        rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffResponse {
            handoff_id: opaque(created.handoff_id.as_uuid()).into(),
            installation_id: opaque(created.installation_id.as_uuid()).into(),
            generation_id: opaque(created.generation_id.as_uuid()).into(),
            route: created.route.to_string(),
            expires_at: timestamp(created.expires_at).into(),
            ..Default::default()
        },
    )
}

async fn audit_handoff_denial(
    service: &ReleaseRpc,
    budget: &request::RequestBudget,
    transport_context: &RequestContext,
    request_context: Option<&rpc_proto::messages::hephaestus::common::v1::RequestContext>,
    actor_id: Option<UserId>,
    transport_error: RpcError,
    reason: UiRequestAuditReason,
) -> connectrpc::ConnectError {
    let _ = request::run_with_budget(
        budget,
        append_handoff_denial(
            service.ui_request_audit.as_ref(),
            transport_context.extensions().get::<UiHandoffAuditMarker>(),
            request_context,
            actor_id,
            reason,
        ),
    )
    .await;
    into_connect_error(transport_error)
}

async fn append_handoff_denial(
    sink: &dyn UiRequestAuditSink,
    marker: Option<&UiHandoffAuditMarker>,
    request_context: Option<&rpc_proto::messages::hephaestus::common::v1::RequestContext>,
    actor_id: Option<UserId>,
    reason: UiRequestAuditReason,
) -> RequestId {
    let request_id = marker.map_or_else(
        || {
            if actor_id.is_some() {
                request_context
                    .and_then(|context| context.request_id.as_option())
                    .and_then(|value| Uuid::parse_str(value.value.trim()).ok())
                    .map_or_else(RequestId::new, RequestId::from_uuid)
            } else {
                RequestId::new()
            }
        },
        UiHandoffAuditMarker::request_id,
    );
    let context = actor_id.map_or_else(
        UiRequestAuditContext::anonymous,
        UiRequestAuditContext::actor,
    );
    let event = release_service::NewUiRequestAuditEvent::now(
        request_id,
        UiRequestAuditSurface::HandoffIssue,
        UiRequestAuditDecision::Denied,
        UiRequestAuditOutcome::NotAttempted,
        reason,
        context,
    );
    let _ = append_ui_request_audit_bounded(sink, event).await;
    request_id
}
