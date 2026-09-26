//! Gateway management and redacted ingress RPC boundary.

mod auth;
mod configure_gateway;
mod conversions;
mod install_release_gateways;
mod mutations;
mod project_metadata;
mod queries;
mod service_log_cursor;
mod service_logs;

use super::{MediatorAuthenticator, MutationReceipts};
use crate::application::gateway::GatewayInstallApplication;
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult};
use control_plane_postgres::ControlPlanePool as PgPool;
use gateway_postgres::{
    PostgresGatewayInstaller, PostgresGatewayManagement, PostgresGatewayServiceLogReader,
};
use rpc_proto::{
    connect::hephaestus::gateway::v1::{GatewayService, GatewayServiceExt},
    messages::hephaestus::gateway::v1::{
        ConfigureGatewayRequest, ConfigureGatewayResponse, CreateMailboxBindingRequest,
        CreateMailboxBindingResponse, GetGatewayRequest, GetGatewayResponse,
        GetProjectServiceLogMetadataRequest, GetProjectServiceLogMetadataResponse,
        InstallReleaseGatewaysRequest, InstallReleaseGatewaysResponse, ListGatewayIngressRequest,
        ListGatewayIngressResponse, ListGatewayServiceLogsRequest, ListGatewayServiceLogsResponse,
        ListMailboxBindingsRequest, ListMailboxBindingsResponse, ListMailboxPublicationsRequest,
        ListMailboxPublicationsResponse, ListProjectGatewaysRequest, ListProjectGatewaysResponse,
        RevokeMailboxBindingGrantRequest, RevokeMailboxBindingGrantResponse,
        SetGatewayLifecycleRequest, SetGatewayLifecycleResponse,
    },
};
use std::sync::Arc;

pub struct GatewayRpc {
    application: PostgresGatewayManagement,
    installer_application: GatewayInstallApplication,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
    service_logs: PostgresGatewayServiceLogReader,
    service_log_cursor: service_log_cursor::ServiceLogCursorCodec,
}

impl GatewayRpc {
    fn new(
        pool: &PgPool,
        application_pool: &PgPool,
        storage: Arc<forge_service::GitStorage>,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
        cursor_key: [u8; 32],
    ) -> Self {
        let authorizer = Arc::new(authz_postgres::PostgresMelangeAuthorizer);
        let installer = PostgresGatewayInstaller::new(pool.clone(), Arc::clone(&authorizer));
        Self {
            application: PostgresGatewayManagement::new(pool.clone(), Arc::clone(&authorizer)),
            installer_application: GatewayInstallApplication::new(installer, storage),
            authenticator,
            receipts,
            service_logs: PostgresGatewayServiceLogReader::new(
                application_pool.clone(),
                Arc::clone(&authorizer),
            ),
            service_log_cursor: service_log_cursor::ServiceLogCursorCodec::new(cursor_key),
        }
    }
}

/// Registers the gateway management service.
pub fn register(
    router: Router,
    pool: &PgPool,
    application_pool: &PgPool,
    storage: Arc<forge_service::GitStorage>,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
    cursor_key: [u8; 32],
) -> Router {
    GatewayServiceExt::register(
        Arc::new(GatewayRpc::new(
            pool,
            application_pool,
            storage,
            authenticator,
            receipts,
            cursor_key,
        )),
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
        queries::list_project_gateways(self, ctx, message).await
    }

    async fn get_gateway(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetGatewayRequest>,
    ) -> ServiceResult<GetGatewayResponse> {
        queries::get_gateway(self, ctx, message).await
    }

    async fn list_gateway_service_logs(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListGatewayServiceLogsRequest>,
    ) -> ServiceResult<ListGatewayServiceLogsResponse> {
        service_logs::handle(self, ctx, message).await
    }

    async fn get_project_service_log_metadata(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, GetProjectServiceLogMetadataRequest>,
    ) -> ServiceResult<GetProjectServiceLogMetadataResponse> {
        project_metadata::handle(self, ctx, message).await
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
        queries::list_gateway_ingress(self, ctx, message).await
    }

    async fn set_gateway_lifecycle(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, SetGatewayLifecycleRequest>,
    ) -> ServiceResult<SetGatewayLifecycleResponse> {
        mutations::set_gateway_lifecycle(self, ctx, message).await
    }

    async fn create_mailbox_binding(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, CreateMailboxBindingRequest>,
    ) -> ServiceResult<CreateMailboxBindingResponse> {
        mutations::create_mailbox_binding(self, ctx, message).await
    }

    async fn revoke_mailbox_binding_grant(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, RevokeMailboxBindingGrantRequest>,
    ) -> ServiceResult<RevokeMailboxBindingGrantResponse> {
        mutations::revoke_mailbox_binding_grant(self, ctx, message).await
    }

    async fn list_mailbox_bindings(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListMailboxBindingsRequest>,
    ) -> ServiceResult<ListMailboxBindingsResponse> {
        queries::list_mailbox_bindings(self, ctx, message).await
    }

    async fn list_mailbox_publications(
        &self,
        ctx: RequestContext,
        message: ServiceRequest<'_, ListMailboxPublicationsRequest>,
    ) -> ServiceResult<ListMailboxPublicationsResponse> {
        queries::list_mailbox_publications(self, ctx, message).await
    }
}
