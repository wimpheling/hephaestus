//! Gateway management and redacted ingress RPC boundary.

mod configure_gateway;
mod install_release_gateways;

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use crate::application::gateway::GatewayInstallApplication;
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use gateway_postgres::{
    GatewayManagementError, GatewayPage, PostgresGatewayInstaller, PostgresGatewayManagement,
};
use rpc_proto::{
    connect::hephaestus::gateway::v1::{GatewayService, GatewayServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest, PageResponse},
        gateway::v1::{
            ConfigureGatewayRequest, ConfigureGatewayResponse, CreateMailboxBindingRequest,
            CreateMailboxBindingResponse, GatewayIngress, GatewayIngressOutcome, GatewayLifecycle,
            GatewayMailboxBinding, GatewayMailboxPublication, GatewayRevision, GatewayRoute,
            GatewaySummary, GetGatewayRequest, GetGatewayResponse, InstallReleaseGatewaysRequest,
            InstallReleaseGatewaysResponse, ListGatewayIngressRequest, ListGatewayIngressResponse,
            ListMailboxBindingsRequest, ListMailboxBindingsResponse,
            ListMailboxPublicationsRequest, ListMailboxPublicationsResponse,
            ListProjectGatewaysRequest, ListProjectGatewaysResponse,
            RevokeMailboxBindingGrantRequest, RevokeMailboxBindingGrantResponse,
            SetGatewayLifecycleRequest, SetGatewayLifecycleResponse,
        },
    },
};
use std::str::FromStr;
use std::sync::Arc;
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

pub struct GatewayRpc {
    application: PostgresGatewayManagement,
    installer_application: GatewayInstallApplication,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl GatewayRpc {
    fn new(
        pool: &PgPool,
        storage: Arc<forge_service::GitStorage>,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
        let installer = PostgresGatewayInstaller::new(pool.clone(), Arc::clone(&authorizer));
        Self {
            application: PostgresGatewayManagement::new(pool.clone(), Arc::clone(&authorizer)),
            installer_application: GatewayInstallApplication::new(installer, storage),
            authenticator,
            receipts,
        }
    }
}

/// Registers the gateway management service.
pub fn register(
    router: Router,
    pool: &PgPool,
    storage: Arc<forge_service::GitStorage>,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
) -> Router {
    GatewayServiceExt::register(
        Arc::new(GatewayRpc::new(pool, storage, authenticator, receipts)),
        router,
    )
}

#[allow(refining_impl_trait)]
impl GatewayService for GatewayRpc {
    async fn list_project_gateways(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListProjectGatewaysRequest>,
    ) -> ServiceResult<ListProjectGatewaysResponse> {
        let identity = query(&ctx, &self.authenticator, "ListProjectGateways")?;
        let request = message.to_owned_message();
        let values = self
            .application
            .list_project(
                &identity,
                id(request.project_id.as_option())?,
                page(request.page.as_option())?,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        Response::ok(ListProjectGatewaysResponse {
            gateways: values.into_iter().map(summary).collect(),
            page: PageResponse {
                stable_order: String::from("id"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn get_gateway(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetGatewayRequest>,
    ) -> ServiceResult<GetGatewayResponse> {
        let identity = query(&ctx, &self.authenticator, "GetGateway")?;
        let request = message.to_owned_message();
        let (gateway, revisions) = self
            .application
            .get(&identity, id(request.gateway_id.as_option())?)
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        Response::ok(GetGatewayResponse {
            gateway: summary(gateway).into(),
            revisions: revisions.into_iter().map(revision).collect(),
            page: PageResponse {
                stable_order: String::from("created_at_desc"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn install_release_gateways(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, InstallReleaseGatewaysRequest>,
    ) -> ServiceResult<InstallReleaseGatewaysResponse> {
        install_release_gateways::handle(self, ctx, message).await
    }

    async fn configure_gateway(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ConfigureGatewayRequest>,
    ) -> ServiceResult<ConfigureGatewayResponse> {
        configure_gateway::handle(self, ctx, message).await
    }

    async fn list_gateway_ingress(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListGatewayIngressRequest>,
    ) -> ServiceResult<ListGatewayIngressResponse> {
        let identity = query(&ctx, &self.authenticator, "ListGatewayIngress")?;
        let request = message.to_owned_message();
        let values = self
            .application
            .ingress(
                &identity,
                id(request.gateway_id.as_option())?,
                page(request.page.as_option())?,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        Response::ok(ListGatewayIngressResponse {
            ingress: values.iter().map(ingress).collect(),
            page: PageResponse {
                stable_order: String::from("id_desc"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn set_gateway_lifecycle(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, SetGatewayLifecycleRequest>,
    ) -> ServiceResult<SetGatewayLifecycleResponse> {
        let request = message.to_owned_message();
        let identity = request::mutation_identity(
            &ctx,
            &self.authenticator,
            "/hephaestus.gateway.v1.GatewayService/SetGatewayLifecycle",
            request.context.as_option(),
        )
        .map_err(into_connect_error)?;
        let gateway_id = id(request.gateway_id.as_option())?;
        let changed = self
            .application
            .transition(
                &identity,
                gateway_id,
                lifecycle(
                    request
                        .expected
                        .as_known()
                        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
                )?,
                lifecycle(
                    request
                        .next
                        .as_known()
                        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?,
                )?,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        if !changed {
            return Err(into_connect_error(RpcError::FailedPrecondition));
        }
        let (gateway, _) = self
            .application
            .get(&identity, gateway_id)
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        let receipt = mutation_receipt(
            &self.receipts,
            identity.idempotency_id,
            identity.user_id,
            "gateway",
            "project",
        )
        .await?;
        Response::ok(SetGatewayLifecycleResponse {
            gateway: summary(gateway).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn create_mailbox_binding(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, CreateMailboxBindingRequest>,
    ) -> ServiceResult<CreateMailboxBindingResponse> {
        let request = message.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "CreateMailboxBinding",
            request.context.as_option(),
        )?;
        let binding = self
            .application
            .create_mailbox_binding(
                &identity,
                id(request.gateway_revision_id.as_option())?,
                &request.slot_key,
                id(request.mailbox_id.as_option())?,
                &request.producer_id,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        let receipt = receipt(&self.receipts, &identity).await?;
        Response::ok(CreateMailboxBindingResponse {
            binding: mailbox_binding(binding).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn revoke_mailbox_binding_grant(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, RevokeMailboxBindingGrantRequest>,
    ) -> ServiceResult<RevokeMailboxBindingGrantResponse> {
        let request = message.to_owned_message();
        let identity = mutation(
            &ctx,
            &self.authenticator,
            "RevokeMailboxBindingGrant",
            request.context.as_option(),
        )?;
        let binding = self
            .application
            .revoke_mailbox_binding_grant(&identity, id(request.binding_id.as_option())?)
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        let receipt = receipt(&self.receipts, &identity).await?;
        Response::ok(RevokeMailboxBindingGrantResponse {
            binding: mailbox_binding(binding).into(),
            receipt: receipt.into(),
            ..Default::default()
        })
    }

    async fn list_mailbox_bindings(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListMailboxBindingsRequest>,
    ) -> ServiceResult<ListMailboxBindingsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListMailboxBindings")?;
        let request = message.to_owned_message();
        let bindings = self
            .application
            .mailbox_bindings(
                &identity,
                id(request.gateway_revision_id.as_option())?,
                page(request.page.as_option())?,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        Response::ok(ListMailboxBindingsResponse {
            bindings: bindings.into_iter().map(mailbox_binding).collect(),
            page: PageResponse {
                stable_order: String::from("id"),
                ..Default::default()
            }
            .into(),
            ..Default::default()
        })
    }

    async fn list_mailbox_publications(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListMailboxPublicationsRequest>,
    ) -> ServiceResult<ListMailboxPublicationsResponse> {
        let identity = query(&ctx, &self.authenticator, "ListMailboxPublications")?;
        let request = message.to_owned_message();
        let publications = self
            .application
            .mailbox_publications(
                &identity,
                id(request.gateway_id.as_option())?,
                page(request.page.as_option())?,
            )
            .await
            .map_err(|error| map_error(&error))
            .map_err(into_connect_error)?;
        Response::ok(ListMailboxPublicationsResponse {
            publications: publications.into_iter().map(mailbox_publication).collect(),
            page: PageResponse {
                stable_order: String::from("id_desc"),
                ..Default::default()
            }
            .into(),
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
        &format!("/hephaestus.gateway.v1.GatewayService/{method}"),
    )
    .map_err(into_connect_error)
}
fn id(value: Option<&OpaqueId>) -> Result<Uuid, connectrpc::ConnectError> {
    value
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))
        .and_then(|value| {
            Uuid::from_str(&value.value).map_err(|_| into_connect_error(RpcError::InvalidArgument))
        })
}
fn page(value: Option<&PageRequest>) -> Result<GatewayPage, connectrpc::ConnectError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |value| {
        if value.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            value.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let after = value
        .filter(|value| !value.page_token.is_empty())
        .map(|value| Uuid::from_str(&value.page_token))
        .transpose()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    Ok(GatewayPage {
        limit: i64::from(size),
        after,
    })
}
const fn map_error(error: &GatewayManagementError) -> RpcError {
    match error {
        GatewayManagementError::Denied => RpcError::PermissionDenied,
        GatewayManagementError::NotFound => RpcError::NotFound,
        GatewayManagementError::InvalidArgument => RpcError::InvalidArgument,
        GatewayManagementError::Conflict => RpcError::FailedPrecondition,
        GatewayManagementError::Unavailable | GatewayManagementError::Persistence(_) => {
            RpcError::Unavailable
        }
    }
}
fn mutation(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
    context: Option<&rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<identity_domain::AuthenticatedIdentity, connectrpc::ConnectError> {
    request::mutation_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.gateway.v1.GatewayService/{method}"),
        context,
    )
    .map_err(into_connect_error)
}
async fn receipt(
    receipts: &MutationReceipts,
    identity: &identity_domain::AuthenticatedIdentity,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, connectrpc::ConnectError>
{
    mutation_receipt(
        receipts,
        identity.idempotency_id,
        identity.user_id,
        "gateway",
        "project",
    )
    .await
}
fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}
fn timestamp(value: OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
    }
}
fn summary(value: gateway_postgres::GatewayManagementSummary) -> GatewaySummary {
    GatewaySummary {
        id: opaque(value.id).into(),
        project_id: opaque(value.project_id).into(),
        repository_id: opaque(value.repository_id).into(),
        name: value.name,
        lifecycle: lifecycle_proto(&value.lifecycle).into(),
        active_revision_id: value.active_revision_id.map(opaque).into(),
        updated_at: timestamp(value.updated_at).into(),
        ..Default::default()
    }
}
fn revision(value: gateway_postgres::GatewayManagementRevision) -> GatewayRevision {
    GatewayRevision {
        id: opaque(value.id).into(),
        release_id: value.release_id.map(opaque).into(),
        release_agent_id: value.release_agent_id.map(opaque).into(),
        handler_contract: value.handler_contract,
        exposure: value.exposure,
        secret_slots: value.secret_slots,
        mailbox_slots: value.mailbox_slots,
        created_at: timestamp(value.created_at).into(),
        routes: value.routes.into_iter().map(route).collect(),
        ..Default::default()
    }
}
fn route(value: gateway_postgres::GatewayManagementRoute) -> GatewayRoute {
    GatewayRoute {
        id: opaque(value.id).into(),
        path: value.path,
        methods: value.methods,
        enabled: value.enabled,
        ..Default::default()
    }
}
fn ingress(value: &gateway_postgres::GatewayIngressSummary) -> GatewayIngress {
    GatewayIngress {
        id: opaque(value.id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        gateway_route_id: opaque(value.gateway_route_id).into(),
        outcome: ingress_outcome(&value.outcome).into(),
        accepted_at: timestamp(value.accepted_at).into(),
        completed_at: value.completed_at.map(timestamp).into(),
        ..Default::default()
    }
}
fn mailbox_binding(value: gateway_postgres::GatewayMailboxBindingSummary) -> GatewayMailboxBinding {
    GatewayMailboxBinding {
        id: opaque(value.id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        mailbox_id: opaque(value.mailbox_id).into(),
        slot_key: value.slot_key,
        producer_id: value.producer_id,
        grant_id: opaque(value.grant_id).into(),
        grant_status: value.grant_status,
        created_at: timestamp(value.created_at).into(),
        granted_at: timestamp(value.granted_at).into(),
        revoked_at: value.revoked_at.map(timestamp).into(),
        ..Default::default()
    }
}
fn mailbox_publication(
    value: gateway_postgres::GatewayMailboxPublicationSummary,
) -> GatewayMailboxPublication {
    GatewayMailboxPublication {
        id: opaque(value.id).into(),
        invocation_id: opaque(value.invocation_id).into(),
        gateway_revision_id: opaque(value.gateway_revision_id).into(),
        binding_id: value.binding_id.map(opaque).into(),
        grant_id: value.grant_id.map(opaque).into(),
        mailbox_id: value.mailbox_id.map(opaque).into(),
        event_id: value.event_id.map(opaque).into(),
        slot_key: value.slot_key,
        outcome: value.outcome,
        accepted_at: timestamp(value.accepted_at).into(),
        settled_at: timestamp(value.settled_at).into(),
        authorization_snapshot_id: value.authorization_snapshot_id.map(opaque).into(),
        snapshot_binding_ordinal: value
            .snapshot_binding_ordinal
            .and_then(|ordinal| u32::try_from(ordinal).ok()),
        delivery_disposition: value.delivery_disposition.unwrap_or_default(),
        delivery_attempt_count: value
            .delivery_attempt_count
            .and_then(|count| u32::try_from(count).ok())
            .unwrap_or_default(),
        delivery_terminal_at: value.delivery_terminal_at.map(timestamp).into(),
        delivery_attempt_id: value.delivery_attempt_id.map(opaque).into(),
        run_id: value.run_id.map(opaque).into(),
        run_state: value.run_state.unwrap_or_default(),
        run_outcome: value.run_outcome.unwrap_or_default(),
        ..Default::default()
    }
}
fn lifecycle(value: GatewayLifecycle) -> Result<&'static str, connectrpc::ConnectError> {
    match value {
        GatewayLifecycle::Enabled => Ok("enabled"),
        GatewayLifecycle::Paused => Ok("paused"),
        GatewayLifecycle::Removed => Ok("removed"),
        GatewayLifecycle::Unspecified => Err(into_connect_error(RpcError::InvalidArgument)),
    }
}
fn lifecycle_proto(value: &str) -> GatewayLifecycle {
    match value {
        "enabled" => GatewayLifecycle::Enabled,
        "paused" => GatewayLifecycle::Paused,
        "removed" => GatewayLifecycle::Removed,
        _ => GatewayLifecycle::Unspecified,
    }
}
fn ingress_outcome(value: &str) -> GatewayIngressOutcome {
    match value {
        "accepted" => GatewayIngressOutcome::Accepted,
        "completed" => GatewayIngressOutcome::Completed,
        "failed" => GatewayIngressOutcome::Failed,
        "timed_out" => GatewayIngressOutcome::TimedOut,
        "rejected" => GatewayIngressOutcome::Rejected,
        _ => GatewayIngressOutcome::Unspecified,
    }
}
