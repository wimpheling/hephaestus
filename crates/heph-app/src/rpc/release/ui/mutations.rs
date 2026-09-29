use super::{
    ActivateUiInstallation, ActivateUiRequest, ActivateUiResponse, DisableUiInstallation,
    DisableUiRequest, DisableUiResponse, INSTALL_AUDIENCE, InstallUi, InstallUiRequest,
    InstallUiResponse, ReleaseRpc, RemoveUiInstallation, RemoveUiRequest, RemoveUiResponse,
    RequestContext, Response, RollbackUiInstallation, RollbackUiRequest, RollbackUiResponse,
    RpcError, ServiceRequest, ServiceResult, TargetReceiptScope, UiInstallationCallerKey,
    expected_generation_id, install_error, installation_id, into_connect_error, lifecycle,
    lifecycle_receipt, mutation_context, mutation_receipt, opaque, organization_id, parse_target,
    release_id, request, ui_key,
};
use identity_domain::RequestId;

/// Handles `InstallUi` after authenticated middleware has populated identity.
pub async fn install(
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
pub async fn activate(
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
pub async fn rollback(
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
pub async fn disable(
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
pub async fn remove(
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
