//! Scoped durable product-event watch service.

// Product-event projection is also used by the outbound event adapter.
#[allow(clippy::redundant_pub_crate)]
pub(crate) mod model;
mod watch;
mod watch_agent_instance;
mod watch_identity;
mod watch_organization;
mod watch_project;
mod watch_repository;
mod watch_run;

use super::MediatorAuthenticator;
use crate::application::event::{EventApplication, EventWakeupSource};
use crate::event_cursor::EventCursorCodec;
use connectrpc::{RequestContext, Router, ServiceRequest, ServiceResult, ServiceStream};
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::event::v1::{ProductEventService, ProductEventServiceExt},
    messages::hephaestus::event::v1::{
        WatchAgentInstanceRequest, WatchAgentInstanceResponse, WatchIdentityRequest,
        WatchIdentityResponse, WatchOrganizationRequest, WatchOrganizationResponse,
        WatchProjectRequest, WatchProjectResponse, WatchRepositoryRequest, WatchRepositoryResponse,
        WatchRunRequest, WatchRunResponse,
    },
};
use std::sync::Arc;

pub(super) struct EventRpc {
    application: EventApplication,
    authenticator: MediatorAuthenticator,
    cursor_codec: EventCursorCodec,
}

impl EventRpc {
    fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        wakeups: Arc<dyn EventWakeupSource>,
        cursor_key: [u8; 32],
    ) -> Self {
        Self {
            application: EventApplication::new(pool, wakeups),
            authenticator,
            cursor_codec: EventCursorCodec::new(cursor_key),
        }
    }
}

pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    wakeups: Arc<dyn EventWakeupSource>,
    cursor_key: [u8; 32],
) -> Router {
    Arc::new(EventRpc::new(pool, authenticator, wakeups, cursor_key)).register(router)
}

#[allow(refining_impl_trait)]
impl ProductEventService for EventRpc {
    async fn watch_identity(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchIdentityRequest>,
    ) -> ServiceResult<ServiceStream<WatchIdentityResponse>> {
        watch_identity::handle(self, ctx, request).await
    }

    async fn watch_organization(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchOrganizationRequest>,
    ) -> ServiceResult<ServiceStream<WatchOrganizationResponse>> {
        watch_organization::handle(self, ctx, request).await
    }

    async fn watch_project(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchProjectRequest>,
    ) -> ServiceResult<ServiceStream<WatchProjectResponse>> {
        watch_project::handle(self, ctx, request).await
    }

    async fn watch_repository(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchRepositoryRequest>,
    ) -> ServiceResult<ServiceStream<WatchRepositoryResponse>> {
        watch_repository::handle(self, ctx, request).await
    }

    async fn watch_run(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchRunRequest>,
    ) -> ServiceResult<ServiceStream<WatchRunResponse>> {
        watch_run::handle(self, ctx, request).await
    }

    async fn watch_agent_instance(
        &self,
        ctx: RequestContext,
        request: ServiceRequest<'_, WatchAgentInstanceRequest>,
    ) -> ServiceResult<ServiceStream<WatchAgentInstanceResponse>> {
        watch_agent_instance::handle(self, ctx, request).await
    }
}
