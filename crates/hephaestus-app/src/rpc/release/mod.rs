//! Release Connect service composition.

mod get_release;
mod list_repository_releases;
mod model;
mod publish_release;
mod set_draft_version;
mod watch_release;

use super::{MediatorAuthenticator, MutationReceipts};
use crate::{
    application::{
        event::{EventApplication, EventWakeupSource},
        release::ReleaseApplication,
    },
    event_cursor::EventCursorCodec,
};
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::connect::hephaestus::release::v1::{ReleaseService, ReleaseServiceExt};
use std::sync::Arc;

pub(super) struct ReleaseRpc {
    application: ReleaseApplication,
    event_application: EventApplication,
    cursor_codec: EventCursorCodec,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl ReleaseRpc {
    const fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
        event_application: EventApplication,
        cursor_codec: EventCursorCodec,
    ) -> Self {
        Self {
            application: ReleaseApplication::new(pool),
            event_application,
            cursor_codec,
            authenticator,
            receipts,
        }
    }
}

/// Registers the generated release service.
pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
    event_wakeups: std::sync::Arc<dyn EventWakeupSource>,
    cursor_key: [u8; 32],
) -> Router {
    Arc::new(ReleaseRpc::new(
        pool.clone(),
        authenticator,
        receipts,
        EventApplication::new(pool, event_wakeups),
        EventCursorCodec::new(cursor_key),
    ))
    .register(router)
}

#[allow(refining_impl_trait)]
impl ReleaseService for ReleaseRpc {
    async fn list_repository_releases(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::ListRepositoryReleasesRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::release::v1::ListRepositoryReleasesResponse,
    > {
        list_repository_releases::handle(self, ctx, request).await
    }

    async fn get_release(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::GetReleaseRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::GetReleaseResponse>
    {
        get_release::handle(self, ctx, request).await
    }

    async fn set_draft_version(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::SetDraftVersionRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::release::v1::SetDraftVersionResponse,
    > {
        set_draft_version::handle(self, ctx, request).await
    }

    async fn publish_release(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::PublishReleaseRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::release::v1::PublishReleaseResponse,
    > {
        publish_release::handle(self, ctx, request).await
    }

    async fn watch_release(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::WatchReleaseRequest,
        >,
    ) -> connectrpc::ServiceResult<
        connectrpc::ServiceStream<
            rpc_proto::messages::hephaestus::release::v1::WatchReleaseResponse,
        >,
    > {
        watch_release::handle(self, ctx, request).await
    }
}

#[cfg(test)]
mod tests {
    use connectrpc_reflection::Reflector;
    use std::sync::Arc;

    #[test]
    fn reflection_exposes_release_service() {
        let reflector = Reflector::from_descriptor_pool(Arc::new(
            rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
        ))
        .expect("reflection index");
        assert!(
            reflector
                .service_names()
                .iter()
                .any(|service| service == "hephaestus.release.v1.ReleaseService")
        );
    }
}
