//! Build Connect service composition.

mod get_build;
mod list_builds;
pub(super) mod model;
mod rebuild_for_verification;
mod request_build;
mod retry_build;
mod stream;
mod stream_build_logs;
mod watch_build;
mod watch_repository_builds;

use super::{MediatorAuthenticator, MutationReceipts};
use crate::{
    application::{
        build::BuildApplication,
        event::{EventApplication, EventWakeupSource},
    },
    event_cursor::EventCursorCodec,
};
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::connect::hephaestus::build::v1::{BuildService, BuildServiceExt};
use std::sync::Arc;

pub(super) struct BuildRpc {
    application: BuildApplication,
    event_application: EventApplication,
    cursor_codec: EventCursorCodec,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl BuildRpc {
    const fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
        event_application: EventApplication,
        cursor_codec: EventCursorCodec,
    ) -> Self {
        Self {
            application: BuildApplication::new(pool),
            event_application,
            cursor_codec,
            authenticator,
            receipts,
        }
    }
}

/// Registers the generated build service.
pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
    event_wakeups: std::sync::Arc<dyn EventWakeupSource>,
    cursor_key: [u8; 32],
) -> Router {
    Arc::new(BuildRpc::new(
        pool.clone(),
        authenticator,
        receipts,
        EventApplication::new(pool, event_wakeups),
        EventCursorCodec::new(cursor_key),
    ))
    .register(router)
}

#[allow(refining_impl_trait)]
impl BuildService for BuildRpc {
    async fn list_builds(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::ListBuildsRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::ListBuildsResponse>
    {
        list_builds::handle(self, ctx, request).await
    }

    async fn get_build(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::GetBuildRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::GetBuildResponse>
    {
        get_build::handle(self, ctx, request).await
    }

    async fn request_build(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::RequestBuildRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::RequestBuildResponse>
    {
        request_build::handle(self, ctx, request).await
    }

    async fn retry_build(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::RetryBuildRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::build::v1::RetryBuildResponse>
    {
        retry_build::handle(self, ctx, request).await
    }

    async fn rebuild_for_verification(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::RebuildForVerificationRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::build::v1::RebuildForVerificationResponse,
    > {
        rebuild_for_verification::handle(self, ctx, request).await
    }

    async fn watch_build(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::WatchBuildRequest,
        >,
    ) -> connectrpc::ServiceResult<
        connectrpc::ServiceStream<rpc_proto::messages::hephaestus::build::v1::WatchBuildResponse>,
    > {
        watch_build::handle(self, ctx, request).await
    }

    async fn watch_repository_builds(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::WatchRepositoryBuildsRequest,
        >,
    ) -> connectrpc::ServiceResult<
        connectrpc::ServiceStream<
            rpc_proto::messages::hephaestus::build::v1::WatchRepositoryBuildsResponse,
        >,
    > {
        watch_repository_builds::handle(self, ctx, request).await
    }

    async fn stream_build_logs(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::build::v1::StreamBuildLogsRequest,
        >,
    ) -> connectrpc::ServiceResult<
        connectrpc::ServiceStream<
            rpc_proto::messages::hephaestus::build::v1::StreamBuildLogsResponse,
        >,
    > {
        stream_build_logs::handle(self, ctx, request).await
    }
}

#[cfg(test)]
mod tests {
    use connectrpc_reflection::Reflector;
    use std::sync::Arc;

    #[test]
    fn reflection_exposes_build_service() {
        let reflector = Reflector::from_descriptor_pool(Arc::new(
            rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
        ))
        .expect("reflection index");
        assert!(
            reflector
                .service_names()
                .iter()
                .any(|service| service == "hephaestus.build.v1.BuildService")
        );
    }
}
