//! Secret metadata and lifecycle RPC adapters.

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use crate::{
    application::commands::{InternalCommand, InternalCommandState, dispatch},
    application::secret::{
        GrantSummary as ApplicationGrant, ImportSummary as ApplicationImport, Page,
        SecretApplication, SecretQueryError, SecretSummary as ApplicationSecret,
    },
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use forge_domain::{ProjectId, RepositoryId};
use identity_domain::OrganizationId;
use rpc_proto::{
    connect::hephaestus::secret::v1::SecretService,
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest, PageResponse},
        secret::v1::{
            AcceptSecretImportRequest, AcceptSecretImportResponse, AuthorityState,
            CreateSecretRequest, CreateSecretResponse, DeliveryMode as ProtoDeliveryMode,
            DeliveryPhase, GetProjectSecretAuthorityRequest, GetProjectSecretAuthorityResponse,
            GrantSecretRequest, GrantSecretResponse, GrantSummary, ImportSummary,
            ListOrganizationSecretGrantsRequest, ListOrganizationSecretGrantsResponse,
            ListOrganizationSecretsRequest, ListOrganizationSecretsResponse,
            ListProjectSecretsRequest, ListProjectSecretsResponse, PurgeSecretRequest,
            PurgeSecretResponse, RevokeSecretRequest, RevokeSecretResponse, RotateSecretRequest,
            RotateSecretResponse, SecretOwner as ProtoSecretOwner,
            SecretPolicy as ProtoSecretPolicy, SecretState, SecretSummary,
            SecretTarget as ProtoSecretTarget, SetSecretEnabledRequest, SetSecretEnabledResponse,
            secret_owner, secret_target,
        },
    },
};
use secret_domain::{
    DeliveryMode, ExecutionPhase, SecretAlias, SecretName, SecretOwner, SecretTarget,
    SecretUsePolicy,
};
use serde_json::Value;
use std::str::FromStr;
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

pub struct SecretRpc {
    application: SecretApplication,
    commands: InternalCommandState,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl SecretRpc {
    pub const fn new(
        pool: PgPool,
        commands: InternalCommandState,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: SecretApplication::new(pool),
            commands,
            authenticator,
            receipts,
        }
    }

    async fn execute(
        &self,
        identity: &identity_domain::AuthenticatedIdentity,
        command: InternalCommand,
    ) -> Result<Value, RpcError> {
        dispatch(&self.commands, identity, command)
            .await
            .map_err(|error| {
                tracing::warn!(
                    actor_id = %identity.user_id,
                    request_id = %identity.request_id,
                    %error,
                    "secret RPC command rejected"
                );
                RpcError::FailedPrecondition
            })
    }
}

#[allow(refining_impl_trait)]
impl SecretService for SecretRpc {
    async fn list_project_secrets(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListProjectSecretsRequest>,
    ) -> ServiceResult<ListProjectSecretsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListProjectSecrets")?;
        let request = request.to_owned_message();
        let result = self
            .application
            .list_project_secrets(
                &identity,
                parse_uuid(request.project_id.as_option())?,
                parse_page(request.page.as_option())?,
            )
            .await
            .map_err(application_error)?;
        Response::ok(ListProjectSecretsResponse {
            page: page_response(result.next_page_token, "name,id").into(),
            secrets: result.values.into_iter().map(secret_summary).collect(),
            ..Default::default()
        })
    }

    async fn list_organization_secrets(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationSecretsRequest>,
    ) -> ServiceResult<ListOrganizationSecretsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListOrganizationSecrets")?;
        let request = request.to_owned_message();
        let result = self
            .application
            .list_organization_secrets(
                &identity,
                parse_uuid(request.organization_id.as_option())?,
                parse_page(request.page.as_option())?,
            )
            .await
            .map_err(application_error)?;
        Response::ok(ListOrganizationSecretsResponse {
            page: page_response(result.next_page_token, "name,id").into(),
            secrets: result.values.into_iter().map(secret_summary).collect(),
            ..Default::default()
        })
    }

    async fn list_organization_secret_grants(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, ListOrganizationSecretGrantsRequest>,
    ) -> ServiceResult<ListOrganizationSecretGrantsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListOrganizationSecretGrants")?;
        let request = request.to_owned_message();
        let result = self
            .application
            .list_organization_grants(
                &identity,
                parse_uuid(request.organization_id.as_option())?,
                parse_page(request.page.as_option())?,
            )
            .await
            .map_err(application_error)?;
        Response::ok(ListOrganizationSecretGrantsResponse {
            page: page_response(result.next_page_token, "secret_name,created_at,id").into(),
            grants: result.values.into_iter().map(grant_summary).collect(),
            ..Default::default()
        })
    }

    async fn get_project_secret_authority(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GetProjectSecretAuthorityRequest>,
    ) -> ServiceResult<GetProjectSecretAuthorityResponse> {
        let identity = query(&ctx, &self.authenticator, "GetProjectSecretAuthority")?;
        let request = request.to_owned_message();
        let authority = self
            .application
            .project_authority(
                &identity,
                parse_uuid(request.project_id.as_option())?,
                parse_page(request.grants_page.as_option())?,
                parse_page(request.imports_page.as_option())?,
            )
            .await
            .map_err(application_error)?;
        let grants_page = page_response(authority.grants.next_page_token, "secret_name,id");
        let imports_page = page_response(authority.imports.next_page_token, "alias,id");
        Response::ok(GetProjectSecretAuthorityResponse {
            grants: authority
                .grants
                .values
                .into_iter()
                .map(grant_summary)
                .collect(),
            imports: authority
                .imports
                .values
                .into_iter()
                .map(import_summary)
                .collect(),
            grants_page: grants_page.into(),
            imports_page: imports_page.into(),
            ..Default::default()
        })
    }

    async fn create_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, CreateSecretRequest>,
    ) -> ServiceResult<CreateSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "CreateSecret", &request.context)?;
        let secret = request
            .secret
            .as_option()
            .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
        let value = self
            .execute(
                &identity,
                InternalCommand::CreateSecret {
                    owner: secret_owner(request.owner.as_option())?,
                    name: SecretName::parse(request.name).map_err(invalid)?,
                    allowed_delivery_modes: delivery_modes(request.allowed_delivery_modes)?,
                    value: secret.value.clone(),
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_metadata",
            "organization",
        )
        .await?;
        Response::ok(CreateSecretResponse {
            secret_id: opaque(json_id(&value, "secret_id")?).into(),
            version_id: opaque(json_id(&value, "secret_version_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn rotate_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RotateSecretRequest>,
    ) -> ServiceResult<RotateSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "RotateSecret", &request.context)?;
        let value = self
            .execute(
                &identity,
                InternalCommand::RotateSecret {
                    secret_id: parse_id(request.secret_id.as_option())?,
                    expected_active_version_id: parse_id(
                        request.expected_active_version_id.as_option(),
                    )?,
                    value: request
                        .secret
                        .as_option()
                        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
                        .value
                        .clone(),
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_metadata",
            "organization",
        )
        .await?;
        Response::ok(RotateSecretResponse {
            secret_id: opaque(json_id(&value, "secret_id")?).into(),
            version_id: opaque(json_id(&value, "secret_version_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn revoke_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, RevokeSecretRequest>,
    ) -> ServiceResult<RevokeSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "RevokeSecret", &request.context)?;
        let secret_id = parse_id(request.secret_id.as_option())?;
        self.execute(&identity, InternalCommand::RevokeSecret { secret_id })
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_metadata",
            "organization",
        )
        .await?;
        Response::ok(RevokeSecretResponse {
            secret_id: opaque(secret_id.to_string()).into(),
            state: SecretState::Revoked.into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn set_secret_enabled(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, SetSecretEnabledRequest>,
    ) -> ServiceResult<SetSecretEnabledResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "SetSecretEnabled",
            &request.context,
        )?;
        let secret_id = parse_id(request.secret_id.as_option())?;
        self.execute(
            &identity,
            InternalCommand::SetSecretEnabled {
                secret_id,
                enabled: request.enabled,
            },
        )
        .await
        .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_metadata",
            "organization",
        )
        .await?;
        Response::ok(SetSecretEnabledResponse {
            secret_id: opaque(secret_id.to_string()).into(),
            state: if request.enabled {
                SecretState::Active
            } else {
                SecretState::Disabled
            }
            .into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn purge_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, PurgeSecretRequest>,
    ) -> ServiceResult<PurgeSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "PurgeSecret", &request.context)?;
        let secret_id = parse_id(request.secret_id.as_option())?;
        self.execute(&identity, InternalCommand::PurgeSecret { secret_id })
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_metadata",
            "organization",
        )
        .await?;
        Response::ok(PurgeSecretResponse {
            secret_id: opaque(secret_id.to_string()).into(),
            state: SecretState::Purged.into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn grant_secret(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, GrantSecretRequest>,
    ) -> ServiceResult<GrantSecretResponse> {
        let request = request.to_owned_message();
        let identity = mutation(&ctx, &self.authenticator, "GrantSecret", &request.context)?;
        let value = self
            .execute(
                &identity,
                InternalCommand::GrantSecret {
                    secret_id: parse_id(request.secret_id.as_option())?,
                    target: secret_target(request.target.as_option())?,
                    policy: secret_policy(request.policy.as_option())?,
                    expires_at: request
                        .expires_at
                        .as_option()
                        .map(from_timestamp)
                        .transpose()?,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_grant",
            "organization",
        )
        .await?;
        Response::ok(GrantSecretResponse {
            grant_id: opaque(json_id(&value, "grant_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn accept_secret_import(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, AcceptSecretImportRequest>,
    ) -> ServiceResult<AcceptSecretImportResponse> {
        let request = request.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "AcceptSecretImport",
            &request.context,
        )?;
        let value = self
            .execute(
                &identity,
                InternalCommand::AcceptSecretImport {
                    grant_id: parse_id(request.grant_id.as_option())?,
                    target: secret_target(request.target.as_option())?,
                    alias: SecretAlias::parse(request.alias).map_err(invalid)?,
                },
            )
            .await
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "secret_import",
            "organization",
        )
        .await?;
        Response::ok(AcceptSecretImportResponse {
            import_id: opaque(json_id(&value, "import_id")?).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }
}

fn query(
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

fn mutation(
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

fn parse_page(value: Option<&PageRequest>) -> Result<Page, connectrpc::ConnectError> {
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

fn parse_uuid(value: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    parse_id(value)
}

fn parse_id<T: FromStr>(value: Option<&OpaqueId>) -> Result<T, connectrpc::ConnectError> {
    request::required_id(value)
        .map_err(into_connect_error)?
        .parse()
        .map_err(invalid)
}

fn secret_owner(value: Option<&ProtoSecretOwner>) -> Result<SecretOwner, connectrpc::ConnectError> {
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

fn secret_target(
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

fn delivery_modes(
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

fn secret_policy(
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

fn from_timestamp(
    value: &buffa_types::google::protobuf::Timestamp,
) -> Result<OffsetDateTime, connectrpc::ConnectError> {
    if !(0..1_000_000_000).contains(&value.nanos) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let time = OffsetDateTime::from_unix_timestamp(value.seconds).map_err(invalid)?;
    time.checked_add(Duration::nanoseconds(i64::from(value.nanos)))
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))
}

fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: i32::try_from(value.nanosecond()).unwrap_or_default(),
        ..Default::default()
    }
}

fn secret_summary(value: ApplicationSecret) -> SecretSummary {
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

fn grant_summary(value: ApplicationGrant) -> GrantSummary {
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

fn import_summary(value: ApplicationImport) -> ImportSummary {
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
        ..Default::default()
    }
}

fn proto_target(kind: &str, id: Uuid) -> ProtoSecretTarget {
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

fn proto_policy(
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

fn proto_mode(value: &str) -> ProtoDeliveryMode {
    match value {
        "raw" => ProtoDeliveryMode::Raw,
        "brokered" => ProtoDeliveryMode::Brokered,
        _ => ProtoDeliveryMode::Unspecified,
    }
}

fn secret_state(value: &str) -> SecretState {
    match value {
        "active" => SecretState::Active,
        "disabled" => SecretState::Disabled,
        "revoked" => SecretState::Revoked,
        "purged" | "tombstoned" => SecretState::Purged,
        _ => SecretState::Unspecified,
    }
}

fn authority_state(value: &str) -> AuthorityState {
    match value {
        "active" => AuthorityState::Active,
        "revoked" => AuthorityState::Revoked,
        "expired" => AuthorityState::Expired,
        _ => AuthorityState::Unspecified,
    }
}

fn page_response(next_page_token: Option<String>, stable_order: &str) -> PageResponse {
    PageResponse {
        next_page_token: next_page_token.unwrap_or_default(),
        stable_order: stable_order.to_owned(),
        ..Default::default()
    }
}

fn application_error(error: SecretQueryError) -> connectrpc::ConnectError {
    match error {
        SecretQueryError::InvalidPage => into_connect_error(RpcError::InvalidArgument),
        SecretQueryError::Persistence(source) => {
            tracing::error!(error = %source, "secret metadata query failed");
            into_connect_error(RpcError::Unavailable)
        }
        SecretQueryError::Unavailable => into_connect_error(RpcError::Unavailable),
    }
}

fn json_id(value: &Value, field: &str) -> Result<String, connectrpc::ConnectError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| into_connect_error(RpcError::Internal))
}

fn opaque(value: String) -> OpaqueId {
    OpaqueId {
        value,
        ..Default::default()
    }
}

fn invalid<T>(_error: T) -> connectrpc::ConnectError {
    into_connect_error(RpcError::InvalidArgument)
}
