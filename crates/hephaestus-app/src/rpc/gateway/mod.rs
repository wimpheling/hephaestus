//! Gateway management and redacted ingress RPC boundary.

use super::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use connectrpc::{RequestContext, Response, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use gateway_postgres::{GatewayManagementError, GatewayPage, PostgresGatewayManagement};
use rpc_proto::{
    connect::hephaestus::gateway::v1::{GatewayService, GatewayServiceExt},
    messages::hephaestus::{
        common::v1::{OpaqueId, PageRequest, PageResponse},
        gateway::v1::{
            GatewayIngress, GatewayIngressOutcome, GatewayLifecycle, GatewayRevision, GatewayRoute,
            GatewaySummary, GetGatewayRequest, GetGatewayResponse, ListGatewayIngressRequest,
            ListGatewayIngressResponse, ListProjectGatewaysRequest, ListProjectGatewaysResponse,
            SetGatewayLifecycleRequest, SetGatewayLifecycleResponse,
        },
    },
};
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

pub struct GatewayRpc {
    application: PostgresGatewayManagement,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl GatewayRpc {
    fn new(pool: PgPool, authenticator: MediatorAuthenticator, receipts: MutationReceipts) -> Self {
        Self {
            application: PostgresGatewayManagement::new(
                pool,
                std::sync::Arc::new(authz_postgres::PostgresMelangeAuthorizer),
            ),
            authenticator,
            receipts,
        }
    }
}

/// Registers the gateway management service.
pub fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
) -> Router {
    GatewayServiceExt::register(
        std::sync::Arc::new(GatewayRpc::new(pool, authenticator, receipts)),
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
        GatewayManagementError::Unavailable | GatewayManagementError::Persistence(_) => {
            RpcError::Unavailable
        }
    }
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
        handler_contract: value.handler_contract,
        exposure: value.exposure,
        secret_slots: value.secret_slots,
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
