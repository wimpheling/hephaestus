use super::{
    OpaqueId, ProtoNavigation, ProtoTarget, ReleaseRpc, RequestContext, RpcError,
    UiBrowserHandoffSecret, UiIcon, UiInstallationCallerKey, UiInstallationContentKind,
    UiInstallationGenerationId, UiInstallationId, UiInstallationNavigation,
    UiInstallationNavigationError, UiInstallationReceiptScope, UiInstallationState,
    UiInstallationTarget, UiPresentation, into_connect_error, mutation_receipt, request,
};
use forge_domain::{OrganizationId, ProjectId, RepositoryId};
use identity_domain::RequestId;
use release_domain::ReleaseId;
use release_service::UiBrowserHandoffError;
use rpc_proto::messages::hephaestus::release::v1::ui_installation_target;
use uuid::Uuid;

pub fn parse_handoff_secret(value: &[u8]) -> Result<UiBrowserHandoffSecret, RpcError> {
    let bytes: [u8; 32] = value.try_into().map_err(|_| RpcError::InvalidArgument)?;
    Ok(UiBrowserHandoffSecret::from_bytes(bytes))
}

pub fn mutation_context(
    ctx: &RequestContext,
    authenticator: &crate::rpc::MediatorAuthenticator,
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

pub fn ui_key(value: String) -> Result<release_domain::ui::UiKey, connectrpc::ConnectError> {
    release_domain::ui::UiKey::parse(value)
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

pub fn expected_generation_id(
    value: Option<&OpaqueId>,
) -> Result<UiInstallationGenerationId, connectrpc::ConnectError> {
    Ok(UiInstallationGenerationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

pub async fn lifecycle_receipt(
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

pub fn parse_target(
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

pub fn organization_id(
    value: Option<&OpaqueId>,
) -> Result<OrganizationId, connectrpc::ConnectError> {
    Ok(OrganizationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

pub fn release_id(value: Option<&OpaqueId>) -> Result<ReleaseId, connectrpc::ConnectError> {
    Ok(ReleaseId::from_uuid(parse_uuid_value(value.ok_or_else(
        || into_connect_error(RpcError::InvalidArgument),
    )?)?))
}

pub fn installation_id(
    value: Option<&OpaqueId>,
) -> Result<UiInstallationId, connectrpc::ConnectError> {
    Ok(UiInstallationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

pub fn generation_id(
    value: Option<&OpaqueId>,
) -> Result<UiInstallationGenerationId, connectrpc::ConnectError> {
    Ok(UiInstallationGenerationId::from_uuid(parse_uuid_value(
        value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
    )?))
}

pub fn parse_uuid_value(value: &OpaqueId) -> Result<Uuid, connectrpc::ConnectError> {
    Uuid::parse_str(value.value.trim()).map_err(|_| into_connect_error(RpcError::InvalidArgument))
}

pub fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

pub fn timestamp(value: time::OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

pub const fn lifecycle(
    value: UiInstallationState,
) -> rpc_proto::messages::hephaestus::release::v1::UiInstallationLifecycle {
    use rpc_proto::messages::hephaestus::release::v1::UiInstallationLifecycle;
    match value {
        UiInstallationState::Enabled => UiInstallationLifecycle::Enabled,
        UiInstallationState::Disabled => UiInstallationLifecycle::Disabled,
        UiInstallationState::Removed => UiInstallationLifecycle::Removed,
    }
}

pub fn navigation(value: &UiInstallationNavigation) -> ProtoNavigation {
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

pub fn proto_target(value: UiInstallationTarget) -> ProtoTarget {
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

pub const fn proto_icon(
    value: UiIcon,
) -> rpc_proto::messages::hephaestus::release::v1::ReleaseUiIcon {
    use rpc_proto::messages::hephaestus::release::v1::ReleaseUiIcon;
    match value {
        UiIcon::App => ReleaseUiIcon::App,
        UiIcon::Chat => ReleaseUiIcon::Chat,
        UiIcon::Code => ReleaseUiIcon::Code,
        UiIcon::Book => ReleaseUiIcon::Book,
        UiIcon::Chart => ReleaseUiIcon::Chart,
    }
}

pub const fn proto_presentation(
    value: UiPresentation,
) -> rpc_proto::messages::hephaestus::release::v1::ReleaseUiPresentation {
    use rpc_proto::messages::hephaestus::release::v1::ReleaseUiPresentation;
    match value {
        UiPresentation::Iframe => ReleaseUiPresentation::Iframe,
        UiPresentation::FullPage => ReleaseUiPresentation::FullPage,
    }
}

pub const fn install_error(value: release_service::UiInstallationError) -> RpcError {
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

pub const fn navigation_error(value: UiInstallationNavigationError) -> RpcError {
    match value {
        UiInstallationNavigationError::InvalidPage => RpcError::InvalidArgument,
        _ => RpcError::Unavailable,
    }
}

pub const fn handoff_error(value: UiBrowserHandoffError) -> RpcError {
    match value {
        UiBrowserHandoffError::PermissionDenied => RpcError::PermissionDenied,
        UiBrowserHandoffError::InvalidRoute => RpcError::InvalidArgument,
        UiBrowserHandoffError::Unavailable => RpcError::Unavailable,
        UiBrowserHandoffError::InvalidOrExpired => RpcError::FailedPrecondition,
    }
}
