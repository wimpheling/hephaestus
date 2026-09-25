//! Installed UI RPC conversion for installation, navigation, and handoff.
//!
//! The module is deliberately an application-port adapter. It does not query
//! `PostgreSQL` or perform tenant preflights; owner and generation authority is
//! rechecked by the release ports in their transactions.

use super::ReleaseRpc;
use crate::rpc::auth::{UiHandoffAuditMarker, append_ui_request_audit_bounded};
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use hmac::{Hmac, Mac as _};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{
    ReleaseId, UiInstallationCallerKey, UiInstallationGenerationId, UiInstallationId,
    UiInstallationState, UiInstallationTarget,
    ui::{UiIcon, UiPresentation},
    ui_browser::{UiBrowserHandoffSecret, UiBrowserRoute},
};
use release_service::{
    ActivateUiInstallation, CreateUiBrowserHandoff, DisableUiInstallation, InstallUi,
    ListUiInstallations, RemoveUiInstallation, RollbackUiInstallation, UiBrowserHandoffError,
    UiInstallationContentKind, UiInstallationNavigation, UiInstallationNavigationError,
    UiInstallationNavigator, UiInstallationPage, UiInstallationReceiptScope,
    UiInstallationTargetFilter, UiRequestAuditContext, UiRequestAuditDecision,
    UiRequestAuditOutcome, UiRequestAuditReason, UiRequestAuditSink, UiRequestAuditSurface,
};
use rpc_proto::messages::hephaestus::{
    common::v1::{OpaqueId, PageResponse},
    release::v1::{
        ActivateUiRequest, ActivateUiResponse, DisableUiRequest, DisableUiResponse,
        InstallUiRequest, InstallUiResponse, ListUiInstallationsRequest,
        ListUiInstallationsResponse, RemoveUiRequest, RemoveUiResponse, RollbackUiRequest,
        RollbackUiResponse, UiInstallationNavigation as ProtoNavigation,
        UiInstallationTarget as ProtoTarget, ui_installation_target,
    },
};
use sha2::Sha256;
use std::str::FromStr;
use uuid::Uuid;

const INSTALL_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/InstallUi";
const LIST_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/ListUiInstallations";
const HANDOFF_AUDIENCE: &str = "/hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff";
const PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;
const CURSOR_TAG_LENGTH: usize = 32;

/// Handles `InstallUi` after authenticated middleware has populated identity.
pub(super) async fn install(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, InstallUiRequest>,
) -> ServiceResult<InstallUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        INSTALL_AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let context = request
        .context
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let caller_key = UiInstallationCallerKey::parse(context.idempotency_key.clone())
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let organization_id = organization_id(request.organization_id.as_option())?;
    let target = parse_target(request.target.as_option(), organization_id)?;
    let release_id = release_id(request.release_id.as_option())?;
    let ui_key = release_domain::ui::UiKey::parse(request.ui_key)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    let result = request::run_with_budget(
        &budget,
        service.ui_installations.install_ui(
            &identity,
            InstallUi {
                caller_key,
                target,
                release_id,
                ui_key,
                expected_organization_id: Some(organization_id),
                acknowledge_repository_git_access: request.acknowledge_repository_git_access,
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(install_error)
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        mutation_receipt(
            &service.receipts,
            RequestId::from_uuid(result.idempotency_id),
            identity.user_id,
            target.aggregate_type(),
            target.primary_scope_kind(),
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(InstallUiResponse {
        installation_id: opaque(result.installation_id.as_uuid()).into(),
        generation_id: opaque(result.generation_id.as_uuid()).into(),
        lifecycle: lifecycle(result.state).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

/// Handles `ActivateUi` and creates a fresh immutable generation.
pub(super) async fn activate(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ActivateUiRequest>,
) -> ServiceResult<ActivateUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let (identity, caller_key) = mutation_context(
        &ctx,
        &service.authenticator,
        "/hephaestus.release.v1.ReleaseService/ActivateUi",
        request.context.as_option(),
    )?;
    let result = request::run_with_budget(
        &budget,
        service.ui_installations.activate_ui_installation(
            &identity,
            ActivateUiInstallation {
                caller_key,
                installation_id: installation_id(request.installation_id.as_option())?,
                expected_generation_id: Some(expected_generation_id(
                    request.expected_generation_id.as_option(),
                )?),
                release_id: release_id(request.release_id.as_option())?,
                ui_key: ui_key(request.ui_key)?,
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(install_error)
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        lifecycle_receipt(
            service,
            &identity,
            result.idempotency_id,
            result.receipt_scope,
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(ActivateUiResponse {
        installation_id: opaque(result.installation_id.as_uuid()).into(),
        generation_id: opaque(result.generation_id.as_uuid()).into(),
        lifecycle: lifecycle(result.state).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

/// Handles `RollbackUi` and creates a fresh generation pinned to the selected release.
pub(super) async fn rollback(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RollbackUiRequest>,
) -> ServiceResult<RollbackUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let (identity, caller_key) = mutation_context(
        &ctx,
        &service.authenticator,
        "/hephaestus.release.v1.ReleaseService/RollbackUi",
        request.context.as_option(),
    )?;
    let result = request::run_with_budget(
        &budget,
        service.ui_installations.rollback_ui_installation(
            &identity,
            RollbackUiInstallation {
                caller_key,
                installation_id: installation_id(request.installation_id.as_option())?,
                expected_generation_id: Some(expected_generation_id(
                    request.expected_generation_id.as_option(),
                )?),
                release_id: release_id(request.release_id.as_option())?,
                ui_key: ui_key(request.ui_key)?,
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(install_error)
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        lifecycle_receipt(
            service,
            &identity,
            result.idempotency_id,
            result.receipt_scope,
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(RollbackUiResponse {
        installation_id: opaque(result.installation_id.as_uuid()).into(),
        generation_id: opaque(result.generation_id.as_uuid()).into(),
        lifecycle: lifecycle(result.state).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

/// Handles `DisableUi` while retaining the current generation.
pub(super) async fn disable(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, DisableUiRequest>,
) -> ServiceResult<DisableUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let (identity, caller_key) = mutation_context(
        &ctx,
        &service.authenticator,
        "/hephaestus.release.v1.ReleaseService/DisableUi",
        request.context.as_option(),
    )?;
    let result = request::run_with_budget(
        &budget,
        service.ui_installations.disable_ui_installation(
            &identity,
            DisableUiInstallation {
                caller_key,
                installation_id: installation_id(request.installation_id.as_option())?,
                expected_generation_id: Some(expected_generation_id(
                    request.expected_generation_id.as_option(),
                )?),
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(install_error)
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        lifecycle_receipt(
            service,
            &identity,
            result.idempotency_id,
            result.receipt_scope,
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(DisableUiResponse {
        installation_id: opaque(result.installation_id.as_uuid()).into(),
        generation_id: opaque(result.generation_id.as_uuid()).into(),
        lifecycle: lifecycle(result.state).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

/// Handles `RemoveUi` while retaining the current generation and history.
pub(super) async fn remove(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RemoveUiRequest>,
) -> ServiceResult<RemoveUiResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let (identity, caller_key) = mutation_context(
        &ctx,
        &service.authenticator,
        "/hephaestus.release.v1.ReleaseService/RemoveUi",
        request.context.as_option(),
    )?;
    let result = request::run_with_budget(
        &budget,
        service.ui_installations.remove_ui_installation(
            &identity,
            RemoveUiInstallation {
                caller_key,
                installation_id: installation_id(request.installation_id.as_option())?,
                expected_generation_id: Some(expected_generation_id(
                    request.expected_generation_id.as_option(),
                )?),
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(install_error)
    .map_err(into_connect_error)?;
    let receipt = request::run_with_budget(
        &budget,
        lifecycle_receipt(
            service,
            &identity,
            result.idempotency_id,
            result.receipt_scope,
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(RemoveUiResponse {
        installation_id: opaque(result.installation_id.as_uuid()).into(),
        generation_id: opaque(result.generation_id.as_uuid()).into(),
        lifecycle: lifecycle(result.state).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

/// Handles `ListUiInstallations` with an explicit organization and target.
pub(super) async fn list(
    service: &ReleaseRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, ListUiInstallationsRequest>,
) -> ServiceResult<ListUiInstallationsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = request.to_owned_message();
    let identity = request::query_identity(&ctx, &service.authenticator, LIST_AUDIENCE)
        .map_err(into_connect_error)?;
    let organization_id = organization_id(request.organization_id.as_option())?;
    let target = parse_target(request.target.as_option(), organization_id)?;
    let page = request
        .page
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let page_size = if page.page_size == 0 {
        PAGE_SIZE
    } else {
        page.page_size
    };
    if !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    // The navigator validates the UUID cursor against organization and target;
    // this transport codec additionally binds actor, tenant, target, method,
    // and stable order before the cursor reaches the application port.
    let after = if page.page_token.is_empty() {
        None
    } else {
        Some(
            service
                .ui_cursor_codec
                .decode(&page.page_token, identity.user_id, organization_id, target)
                .map_err(into_connect_error)?,
        )
    };
    let result = request::run_with_budget(
        &budget,
        service.ui_navigator.list_ui_installations(
            &identity,
            ListUiInstallations {
                organization_id,
                target: target.filter(),
                page: UiInstallationPage {
                    size: i64::from(page_size),
                    after,
                },
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(navigation_error)
    .map_err(into_connect_error)?;
    let next_page_token = result.next.map(|id| {
        service
            .ui_cursor_codec
            .encode(id, identity.user_id, organization_id, target)
    });
    Response::ok(ListUiInstallationsResponse {
        installations: result.installations.iter().map(navigation).collect(),
        page: PageResponse {
            next_page_token: next_page_token.unwrap_or_default(),
            stable_order: String::from("created_at desc,id desc"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

/// Handles the non-replayable public handoff issuance boundary.
// Keep validation and audit-denial ordering in one boundary so every rejected
// field carries the same verified actor and transport correlation.
#[allow(clippy::too_many_lines)]
pub(super) async fn handoff(
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

fn parse_handoff_secret(value: &[u8]) -> Result<UiBrowserHandoffSecret, RpcError> {
    let bytes: [u8; 32] = value.try_into().map_err(|_| RpcError::InvalidArgument)?;
    Ok(UiBrowserHandoffSecret::from_bytes(bytes))
}

fn mutation_context(
    ctx: &RequestContext,
    authenticator: &super::MediatorAuthenticator,
    audience: &str,
    context: Option<&rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<
    (
        identity_domain::AuthenticatedIdentity,
        UiInstallationCallerKey,
    ),
    connectrpc::ConnectError,
> {
    let identity = request::mutation_identity(ctx, authenticator, audience, context)
        .map_err(into_connect_error)?;
    let context = context.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let caller_key = UiInstallationCallerKey::parse(context.idempotency_key.clone())
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    Ok((identity, caller_key))
}

fn ui_key(value: String) -> Result<release_domain::ui::UiKey, connectrpc::ConnectError> {
    release_domain::ui::UiKey::parse(value)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

fn expected_generation_id(
    value: Option<&OpaqueId>,
) -> Result<UiInstallationGenerationId, connectrpc::ConnectError> {
    Ok(UiInstallationGenerationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

async fn lifecycle_receipt(
    service: &ReleaseRpc,
    identity: &identity_domain::AuthenticatedIdentity,
    idempotency_id: Uuid,
    scope: UiInstallationReceiptScope,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    mutation_receipt(
        &service.receipts,
        RequestId::from_uuid(idempotency_id),
        identity.user_id,
        scope.aggregate_type,
        scope.primary_scope_kind,
    )
    .await
}

fn parse_target(
    value: Option<&ProtoTarget>,
    organization_id: OrganizationId,
) -> Result<UiInstallationTarget, connectrpc::ConnectError> {
    let target = value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    match target.target.as_ref() {
        Some(ui_installation_target::Target::Global(_)) => {
            Ok(UiInstallationTarget::Organization(organization_id))
        }
        Some(ui_installation_target::Target::ProjectId(id)) => Ok(UiInstallationTarget::Project(
            ProjectId::from_uuid(parse_uuid_value(id)?),
        )),
        Some(ui_installation_target::Target::RepositoryId(id)) => Ok(
            UiInstallationTarget::Repository(RepositoryId::from_uuid(parse_uuid_value(id)?)),
        ),
        None => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}

fn organization_id(value: Option<&OpaqueId>) -> Result<OrganizationId, connectrpc::ConnectError> {
    Ok(OrganizationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

fn release_id(value: Option<&OpaqueId>) -> Result<ReleaseId, connectrpc::ConnectError> {
    Ok(ReleaseId::from_uuid(parse_uuid_value(value.ok_or_else(
        || into_connect_error(RpcError::InvalidArgument),
    )?)?))
}

fn installation_id(value: Option<&OpaqueId>) -> Result<UiInstallationId, connectrpc::ConnectError> {
    Ok(UiInstallationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

fn generation_id(
    value: Option<&OpaqueId>,
) -> Result<UiInstallationGenerationId, connectrpc::ConnectError> {
    Ok(UiInstallationGenerationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

fn parse_uuid_value(value: &OpaqueId) -> Result<Uuid, connectrpc::ConnectError> {
    Uuid::parse_str(value.value.trim()).map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn timestamp(value: time::OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

const fn lifecycle(
    value: UiInstallationState,
) -> rpc_proto::messages::hephaestus::release::v1::UiInstallationLifecycle {
    use rpc_proto::messages::hephaestus::release::v1::UiInstallationLifecycle;
    match value {
        UiInstallationState::Enabled => UiInstallationLifecycle::Enabled,
        UiInstallationState::Disabled => UiInstallationLifecycle::Disabled,
        UiInstallationState::Removed => UiInstallationLifecycle::Removed,
    }
}

fn navigation(value: &UiInstallationNavigation) -> ProtoNavigation {
    use rpc_proto::messages::hephaestus::release::v1::UiInstallationContentKind as ProtoContentKind;
    ProtoNavigation {
        installation_id: opaque(value.installation_id.as_uuid()).into(),
        generation_id: opaque(value.generation_id.as_uuid()).into(),
        organization_id: opaque(value.organization_id.as_uuid()).into(),
        target: Some(proto_target(value.target)).into(),
        lifecycle: lifecycle(value.lifecycle).into(),
        release_id: opaque(value.release_id.as_uuid()).into(),
        ui_key: value.ui_key.to_string(),
        label: value.label.to_string(),
        icon: proto_icon(value.icon).into(),
        presentation: proto_presentation(value.presentation).into(),
        route_base: value.route_base.to_string(),
        content_kind: match value.content_kind {
            UiInstallationContentKind::Static => ProtoContentKind::Static,
            UiInstallationContentKind::ManagedService => ProtoContentKind::ManagedService,
        }
        .into(),
        launchable: value.launchable,
        ..Default::default()
    }
}

fn proto_target(value: UiInstallationTarget) -> ProtoTarget {
    match value {
        UiInstallationTarget::Organization(_) => ProtoTarget {
            target: Some(ui_installation_target::Target::Global(Box::default())),
            ..Default::default()
        },
        UiInstallationTarget::Project(id) => ProtoTarget {
            target: Some(ui_installation_target::Target::ProjectId(Box::new(opaque(
                id.as_uuid(),
            )))),
            ..Default::default()
        },
        UiInstallationTarget::Repository(id) => ProtoTarget {
            target: Some(ui_installation_target::Target::RepositoryId(Box::new(
                opaque(id.as_uuid()),
            ))),
            ..Default::default()
        },
    }
}

const fn proto_icon(value: UiIcon) -> rpc_proto::messages::hephaestus::release::v1::ReleaseUiIcon {
    use rpc_proto::messages::hephaestus::release::v1::ReleaseUiIcon;
    match value {
        UiIcon::App => ReleaseUiIcon::App,
        UiIcon::Chat => ReleaseUiIcon::Chat,
        UiIcon::Code => ReleaseUiIcon::Code,
        UiIcon::Book => ReleaseUiIcon::Book,
        UiIcon::Chart => ReleaseUiIcon::Chart,
    }
}

const fn proto_presentation(
    value: UiPresentation,
) -> rpc_proto::messages::hephaestus::release::v1::ReleaseUiPresentation {
    use rpc_proto::messages::hephaestus::release::v1::ReleaseUiPresentation;
    match value {
        UiPresentation::Iframe => ReleaseUiPresentation::Iframe,
        UiPresentation::FullPage => ReleaseUiPresentation::FullPage,
    }
}

const fn install_error(value: release_service::UiInstallationError) -> RpcError {
    match value {
        release_service::UiInstallationError::PermissionDenied => RpcError::PermissionDenied,
        release_service::UiInstallationError::OrganizationMismatch => RpcError::InvalidArgument,
        release_service::UiInstallationError::AlreadyInstalled => RpcError::AlreadyExists,
        release_service::UiInstallationError::InvalidOrUnsupported
        | release_service::UiInstallationError::IdempotencyConflict
        | release_service::UiInstallationError::GenerationConflict
        | release_service::UiInstallationError::InvalidTransition => RpcError::FailedPrecondition,
        _ => RpcError::Unavailable,
    }
}

const fn navigation_error(value: UiInstallationNavigationError) -> RpcError {
    match value {
        UiInstallationNavigationError::InvalidPage => RpcError::InvalidArgument,
        _ => RpcError::Unavailable,
    }
}

const fn handoff_error(value: UiBrowserHandoffError) -> RpcError {
    match value {
        UiBrowserHandoffError::PermissionDenied => RpcError::PermissionDenied,
        UiBrowserHandoffError::InvalidRoute => RpcError::InvalidArgument,
        UiBrowserHandoffError::Unavailable => RpcError::Unavailable,
        UiBrowserHandoffError::InvalidOrExpired => RpcError::FailedPrecondition,
    }
}

/// Signed cursor bound to actor, organization, exact target, method, and order.
#[derive(Clone)]
pub(super) struct UiInstallationCursorCodec {
    key: [u8; 32],
}

impl UiInstallationCursorCodec {
    pub(super) const fn new(key: [u8; 32]) -> Self {
        Self { key }
    }
    fn encode(
        &self,
        id: UiInstallationId,
        actor: UserId,
        organization: OrganizationId,
        target: UiInstallationTarget,
    ) -> String {
        let (kind, target_id) = cursor_scope(target);
        let payload = format!(
            "v1|{actor}|{organization}|{kind}|{target_id}|{}|created_at-desc-id-desc",
            id.as_uuid()
        );
        let tag = self.tag(payload.as_bytes());
        let mut bytes = payload.into_bytes();
        bytes.push(b'|');
        bytes.extend_from_slice(&tag);
        URL_SAFE_NO_PAD.encode(bytes)
    }
    fn decode(
        &self,
        token: &str,
        actor: UserId,
        organization: OrganizationId,
        target: UiInstallationTarget,
    ) -> Result<UiInstallationId, RpcError> {
        if token.is_empty() || token.len() > 512 || !token.is_ascii() {
            return Err(RpcError::InvalidArgument);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(token)
            .map_err(|_| RpcError::InvalidArgument)?;
        let delimiter = bytes
            .len()
            .checked_sub(CURSOR_TAG_LENGTH + 1)
            .ok_or(RpcError::InvalidArgument)?;
        if bytes.get(delimiter) != Some(&b'|') {
            return Err(RpcError::InvalidArgument);
        }
        let payload = &bytes[..delimiter];
        let supplied = &bytes[delimiter + 1..];
        if !self.verify_tag(payload, supplied) {
            return Err(RpcError::InvalidArgument);
        }
        let fields: Vec<&str> = std::str::from_utf8(payload)
            .map_err(|_| RpcError::InvalidArgument)?
            .split('|')
            .collect();
        let (kind, target_id) = cursor_scope(target);
        if fields.len() != 7
            || fields[0] != "v1"
            || fields[1] != actor.to_string()
            || fields[2] != organization.to_string()
            || fields[3] != kind
            || fields[4] != target_id.to_string()
            || fields[6] != "created_at-desc-id-desc"
        {
            return Err(RpcError::InvalidArgument);
        }
        Uuid::parse_str(fields[5])
            .map(UiInstallationId::from_uuid)
            .map_err(|_| RpcError::InvalidArgument)
    }
    fn tag(&self, payload: &[u8]) -> [u8; 32] {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a 32-byte key");
        mac.update(b"hephaestus-ui-installation-cursor-v1\0");
        mac.update(payload);
        mac.finalize().into_bytes().into()
    }

    fn verify_tag(&self, payload: &[u8], supplied: &[u8]) -> bool {
        let mut mac =
            Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts a 32-byte key");
        mac.update(b"hephaestus-ui-installation-cursor-v1\0");
        mac.update(payload);
        mac.verify_slice(supplied).is_ok()
    }
}
const fn cursor_scope(target: UiInstallationTarget) -> (&'static str, Uuid) {
    match target {
        UiInstallationTarget::Organization(id) => ("global", id.as_uuid()),
        UiInstallationTarget::Project(id) => ("project", id.as_uuid()),
        UiInstallationTarget::Repository(id) => ("repository", id.as_uuid()),
    }
}

trait TargetReceiptScope {
    fn aggregate_type(self) -> &'static str;
    fn primary_scope_kind(self) -> &'static str;
    fn filter(self) -> UiInstallationTargetFilter;
}

impl TargetReceiptScope for UiInstallationTarget {
    fn aggregate_type(self) -> &'static str {
        match self {
            Self::Organization(_) => "organization",
            Self::Project(_) => "project",
            Self::Repository(_) => "repository",
        }
    }
    fn primary_scope_kind(self) -> &'static str {
        match self {
            Self::Repository(_) => "project",
            Self::Organization(_) | Self::Project(_) => self.aggregate_type(),
        }
    }
    fn filter(self) -> UiInstallationTargetFilter {
        match self {
            Self::Organization(id) => {
                let _ = id;
                UiInstallationTargetFilter::Global
            }
            Self::Project(id) => UiInstallationTargetFilter::Project(id),
            Self::Repository(id) => UiInstallationTargetFilter::Repository(id),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_uuid_value;
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use uuid::Uuid;

    #[test]
    fn malformed_opaque_ids_fail_closed() {
        assert!(
            parse_uuid_value(&OpaqueId {
                value: String::from("not-a-uuid"),
                ..Default::default()
            })
            .is_err()
        );
        assert!(
            parse_uuid_value(&OpaqueId {
                value: Uuid::new_v4().to_string(),
                ..Default::default()
            })
            .is_ok()
        );
    }
}

#[cfg(test)]
mod handoff_audit_tests {
    use super::parse_handoff_secret;

    #[test]
    fn malformed_secret_is_rejected_before_store_call() {
        assert!(parse_handoff_secret(&[0_u8; 31]).is_err());
    }
}

#[cfg(test)]
mod receipt_scope_tests {
    use super::TargetReceiptScope;
    use forge_domain::{OrganizationId, ProjectId, RepositoryId};
    use release_domain::UiInstallationTarget;
    use uuid::Uuid;

    #[test]
    fn owner_event_receipt_scopes_match_append_owner_event() {
        let organization = OrganizationId::from_uuid(Uuid::from_u128(1));
        let project = ProjectId::from_uuid(Uuid::from_u128(2));
        let repository = RepositoryId::from_uuid(Uuid::from_u128(3));
        assert_eq!(
            (
                UiInstallationTarget::Organization(organization).aggregate_type(),
                UiInstallationTarget::Organization(organization).primary_scope_kind()
            ),
            ("organization", "organization")
        );
        assert_eq!(
            (
                UiInstallationTarget::Project(project).aggregate_type(),
                UiInstallationTarget::Project(project).primary_scope_kind()
            ),
            ("project", "project")
        );
        assert_eq!(
            (
                UiInstallationTarget::Repository(repository).aggregate_type(),
                UiInstallationTarget::Repository(repository).primary_scope_kind()
            ),
            ("repository", "project")
        );
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::UiInstallationCursorCodec;
    use forge_domain::{OrganizationId, ProjectId};
    use identity_domain::UserId;
    use release_domain::{UiInstallationId, UiInstallationTarget};
    use uuid::Uuid;

    #[test]
    fn cursor_round_trip_and_scope_binding() {
        let codec = UiInstallationCursorCodec::new([7; 32]);
        let actor = UserId::from_uuid(Uuid::from_u128(1));
        let organization = OrganizationId::from_uuid(Uuid::from_u128(2));
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let target = UiInstallationTarget::Project(project);
        let installation = UiInstallationId::from_uuid(Uuid::from_u128(4));
        let token = codec.encode(installation, actor, organization, target);
        assert_eq!(
            codec.decode(&token, actor, organization, target),
            Ok(installation)
        );
        assert_eq!(
            codec.decode(
                &token,
                UserId::from_uuid(Uuid::from_u128(5)),
                organization,
                target
            ),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
        assert_eq!(
            codec.decode(
                &token,
                actor,
                organization,
                UiInstallationTarget::Organization(organization)
            ),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
        let mut tampered = token.into_bytes();
        let last = tampered.last_mut().expect("encoded cursor");
        *last = if *last == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).expect("ASCII cursor");
        assert_eq!(
            codec.decode(&tampered, actor, organization, target),
            Err(crate::rpc::RpcError::InvalidArgument)
        );
    }

    #[test]
    fn cursor_tag_delimiter_does_not_change_fixed_boundary() {
        let codec = UiInstallationCursorCodec::new([7; 32]);
        let actor = UserId::from_uuid(Uuid::from_u128(1));
        let organization = OrganizationId::from_uuid(Uuid::from_u128(2));
        let project = ProjectId::from_uuid(Uuid::from_u128(3));
        let target = UiInstallationTarget::Project(project);
        let installation = (0..1_000_u128)
            .map(|value| UiInstallationId::from_uuid(Uuid::from_u128(value)))
            .find(|id| {
                let (kind, target_id) = super::cursor_scope(target);
                let payload = format!(
                    "v1|{actor}|{organization}|{kind}|{target_id}|{}|created_at-desc-id-desc",
                    id.as_uuid()
                );
                codec.tag(payload.as_bytes()).contains(&b'|')
            })
            .expect("test range includes an HMAC tag containing the delimiter");
        let token = codec.encode(installation, actor, organization, target);
        assert_eq!(
            codec.decode(&token, actor, organization, target),
            Ok(installation)
        );
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::{expected_generation_id, install_error};
    use release_service::UiInstallationError;
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use uuid::Uuid;

    #[test]
    fn lifecycle_cas_is_required_and_uuid_validated() {
        assert!(expected_generation_id(None).is_err());
        assert!(expected_generation_id(Some(&OpaqueId::default())).is_err());
        assert!(
            expected_generation_id(Some(&OpaqueId {
                value: Uuid::new_v4().to_string(),
                ..Default::default()
            }))
            .is_ok()
        );
    }

    #[test]
    fn lifecycle_conflicts_are_failed_preconditions() {
        assert_eq!(
            super::super::super::RpcError::FailedPrecondition,
            install_error(UiInstallationError::GenerationConflict)
        );
        assert_eq!(
            super::super::super::RpcError::FailedPrecondition,
            install_error(UiInstallationError::InvalidTransition)
        );
        assert_eq!(
            super::super::super::RpcError::PermissionDenied,
            install_error(UiInstallationError::PermissionDenied)
        );
    }
}
