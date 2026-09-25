use super::{MediatorAuthenticator, RpcError};
use crate::application::secret::{
    GrantSummary as ApplicationGrant, ImportSummary as ApplicationImport, Page, SecretQueryError,
    SecretSummary as ApplicationSecret,
};
use crate::rpc::{into_connect_error, request};
use connectrpc::RequestContext;
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::OrganizationId;
use rpc_proto::messages::hephaestus::{
    common::v1::{OpaqueId, PageRequest, PageResponse},
    secret::v1::{
        AuthorityState, DeliveryMode as ProtoDeliveryMode, DeliveryPhase, GrantSummary,
        ImportSummary, SecretOwner as ProtoSecretOwner, SecretPolicy as ProtoSecretPolicy,
        SecretState, SecretSummary, SecretTarget as ProtoSecretTarget, secret_owner, secret_target,
    },
};
use secret_domain::{DeliveryMode, ExecutionPhase, SecretOwner, SecretTarget, SecretUsePolicy};
use serde_json::Value;
use std::str::FromStr;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

pub(super) fn query(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::query_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.secret.v1.SecretService/{method}"),
    )
    .map_err(into_connect_error)
}

pub(super) fn mutation(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
    context: &buffa::MessageField<rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::mutation_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.secret.v1.SecretService/{method}"),
        context.as_option(),
    )
    .map_err(into_connect_error)
}

pub(super) fn parse_page(value: Option<&PageRequest>) -> Result<Page, connectrpc::ConnectError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |page| {
        if page.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            page.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let after = value
        .filter(|page| !page.page_token.is_empty())
        .map(|page| Uuid::parse_str(&page.page_token))
        .transpose()
        .map_err(invalid)?;
    Ok(Page {
        size: i64::from(size),
        after,
    })
}

pub(super) fn parse_uuid(value: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    parse_id(value)
}

pub(super) fn parse_id<T: FromStr>(
    value: Option<&OpaqueId>,
) -> Result<T, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(invalid)
}

pub(super) fn secret_owner(
    value: Option<&ProtoSecretOwner>,
) -> Result<SecretOwner, connectrpc::ConnectError> {
    match value.and_then(|owner| owner.owner.as_ref()) {
        Some(secret_owner::Owner::OrganizationId(id)) => Ok(SecretOwner::Organization(
            OrganizationId::from_str(&id.value).map_err(invalid)?,
        )),
        Some(secret_owner::Owner::ProjectId(id)) => Ok(SecretOwner::Project(
            ProjectId::from_str(&id.value).map_err(invalid)?,
        )),
        None => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}

pub(super) fn secret_target(
    value: Option<&ProtoSecretTarget>,
) -> Result<SecretTarget, connectrpc::ConnectError> {
    match value.and_then(|target| target.target.as_ref()) {
        Some(secret_target::Target::ProjectId(id)) => Ok(SecretTarget::Project(
            ProjectId::from_str(&id.value).map_err(invalid)?,
        )),
        Some(secret_target::Target::RepositoryId(id)) => Ok(SecretTarget::Repository(
            RepositoryId::from_str(&id.value).map_err(invalid)?,
        )),
        None => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}

pub(super) fn delivery_modes(
    values: Vec<buffa::EnumValue<ProtoDeliveryMode>>,
) -> Result<Vec<DeliveryMode>, connectrpc::ConnectError> {
    values
        .into_iter()
        .map(|value| match value.as_known() {
            Some(ProtoDeliveryMode::Raw) => Ok(DeliveryMode::Raw),
            Some(ProtoDeliveryMode::Brokered) => Ok(DeliveryMode::Brokered),
            _ => Err(into_connect_error(RpcError::InvalidArgument)),
        })
        .collect()
}

pub(super) fn secret_policy(
    value: Option<&ProtoSecretPolicy>,
) -> Result<SecretUsePolicy, connectrpc::ConnectError> {
    let value = value.ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let phases = value
        .phases
        .iter()
        .map(|phase| match phase.as_known() {
            Some(DeliveryPhase::Normal) => Ok(ExecutionPhase::Normal),
            Some(DeliveryPhase::Update) => Ok(ExecutionPhase::Update),
            _ => Err(into_connect_error(RpcError::InvalidArgument)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    SecretUsePolicy {
        delivery_modes: delivery_modes(value.delivery_modes.clone())?,
        phases,
        destinations: value.destinations.clone(),
    }
    .normalized()
    .map_err(invalid)
}

pub(super) fn from_timestamp(
    value: &buffa_types::google::protobuf::Timestamp,
) -> Result<OffsetDateTime, connectrpc::ConnectError> {
    if !(0..1_000_000_000).contains(&value.nanos) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let time = OffsetDateTime::from_unix_timestamp(value.seconds).map_err(invalid)?;
    time.checked_add(Duration::nanoseconds(i64::from(value.nanos)))
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))
}

pub(super) fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

pub(super) fn secret_summary(value: ApplicationSecret) -> SecretSummary {
    SecretSummary {
        id: opaque(value.id.to_string()).into(),
        name: value.name,
        state: secret_state(&value.status).into(),
        allowed_delivery_modes: value
            .allowed_delivery_modes
            .iter()
            .map(|mode| proto_mode(mode).into())
            .collect(),
        active_version_id: value
            .active_version_id
            .map(|id| opaque(id.to_string()))
            .into(),
        created_at: timestamp(value.created_at).into(),
        updated_at: timestamp(value.updated_at).into(),
        active_version_sequence: value.active_version_sequence.unwrap_or_default(),
        active_version_created_at: value.active_version_created_at.map(timestamp).into(),
        grant_count: value.grant_count,
        import_count: value.import_count,
        binding_count: value.binding_count,
        has_raw_binding: value.has_raw_binding,
        can_rotate: value.can_rotate,
        can_manage_grants: value.can_manage_grants,
        can_revoke: value.can_revoke,
        can_purge: value.can_purge,
        ..Default::default()
    }
}

pub(super) fn grant_summary(value: ApplicationGrant) -> GrantSummary {
    GrantSummary {
        id: opaque(value.id.to_string()).into(),
        secret_id: opaque(value.secret_id.to_string()).into(),
        secret_name: value.secret_name,
        target: proto_target(&value.target_kind, value.target_id).into(),
        target_name: value.target_name.unwrap_or_default(),
        policy: proto_policy(&value.delivery_modes, &value.phases, value.destinations).into(),
        expires_at: value.expires_at.map(timestamp).into(),
        state: authority_state(&value.status).into(),
        created_at: timestamp(value.created_at).into(),
        import_count: value.import_count,
        import_id: value.import_id.map(|id| opaque(id.to_string())).into(),
        import_alias: value.import_alias.unwrap_or_default(),
        import_state: value
            .import_status
            .as_deref()
            .map_or(AuthorityState::Unspecified, authority_state)
            .into(),
        ..Default::default()
    }
}

pub(super) fn import_summary(value: ApplicationImport) -> ImportSummary {
    ImportSummary {
        id: opaque(value.id.to_string()).into(),
        alias: value.alias,
        target: proto_target(&value.target_kind, value.target_id).into(),
        state: authority_state(&value.status).into(),
        secret_id: opaque(value.secret_id.to_string()).into(),
        secret_name: value.secret_name,
        secret_state: secret_state(&value.secret_status).into(),
        policy: proto_policy(&value.delivery_modes, &value.phases, value.destinations).into(),
        expires_at: value.expires_at.map(timestamp).into(),
        active_version_id: value
            .active_version_id
            .map(|id| opaque(id.to_string()))
            .into(),
        ..Default::default()
    }
}

pub(super) fn proto_target(kind: &str, id: Uuid) -> ProtoSecretTarget {
    let id = opaque(id.to_string());
    let target = match kind {
        "project" => secret_target::Target::ProjectId(Box::new(id)),
        "repository" => secret_target::Target::RepositoryId(Box::new(id)),
        _ => return ProtoSecretTarget::default(),
    };
    ProtoSecretTarget {
        target: Some(target),
        ..Default::default()
    }
}

pub(super) fn proto_policy(
    modes: &[String],
    phases: &[String],
    destinations: Vec<String>,
) -> ProtoSecretPolicy {
    ProtoSecretPolicy {
        delivery_modes: modes.iter().map(|mode| proto_mode(mode).into()).collect(),
        phases: phases
            .iter()
            .map(|phase| {
                match phase.as_str() {
                    "normal" => DeliveryPhase::Normal,
                    "update" => DeliveryPhase::Update,
                    _ => DeliveryPhase::Unspecified,
                }
                .into()
            })
            .collect(),
        destinations,
        ..Default::default()
    }
}

pub(super) fn proto_mode(value: &str) -> ProtoDeliveryMode {
    match value {
        "raw" => ProtoDeliveryMode::Raw,
        "brokered" => ProtoDeliveryMode::Brokered,
        _ => ProtoDeliveryMode::Unspecified,
    }
}

pub(super) fn secret_state(value: &str) -> SecretState {
    match value {
        "active" => SecretState::Active,
        "disabled" => SecretState::Disabled,
        "revoked" => SecretState::Revoked,
        "purged" | "tombstoned" => SecretState::Purged,
        _ => SecretState::Unspecified,
    }
}

pub(super) fn authority_state(value: &str) -> AuthorityState {
    match value {
        "active" => AuthorityState::Active,
        "revoked" => AuthorityState::Revoked,
        "expired" => AuthorityState::Expired,
        _ => AuthorityState::Unspecified,
    }
}

pub(super) fn page_response(next_page_token: Option<String>, stable_order: &str) -> PageResponse {
    PageResponse {
        next_page_token: next_page_token.unwrap_or_default(),
        stable_order: stable_order.to_owned(),
        ..Default::default()
    }
}

pub(super) fn application_error(error: SecretQueryError) -> connectrpc::ConnectError {
    match error {
        SecretQueryError::InvalidPage => into_connect_error(RpcError::InvalidArgument),
        SecretQueryError::Persistence(source) => {
            tracing::error!(error = %source, "secret metadata query failed");
            into_connect_error(RpcError::Unavailable)
        }
        SecretQueryError::Unavailable => into_connect_error(RpcError::Unavailable),
    }
}

pub(super) fn json_id(value: &Value, field: &str) -> Result<String, connectrpc::ConnectError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| into_connect_error(RpcError::Internal))
}

pub(super) fn opaque(value: String) -> OpaqueId {
    OpaqueId {
        value,
        ..Default::default()
    }
}

pub(super) fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}
