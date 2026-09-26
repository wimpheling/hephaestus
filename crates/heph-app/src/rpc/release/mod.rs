//! Release Connect service composition.

mod get_release;
mod list_repository_releases;
mod model;
mod publish_release;
mod set_draft_version;
pub(super) mod ui;
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
    pub(super) ui_installations: Arc<release_postgres::ReleaseService>,
    pub(super) ui_navigator: Arc<release_postgres::PgUiInstallationNavigator>,
    pub(super) ui_browser: Arc<release_postgres::PgUiBrowserSessionStore>,
    pub(super) ui_request_audit: Arc<dyn release_service::UiRequestAuditSink>,
    ui_cursor_codec: ui::UiInstallationCursorCodec,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl ReleaseRpc {
    // Composition wires the existing release/event dependencies and the UI
    // ports; keeping them explicit preserves their ownership boundaries.
    #[allow(clippy::too_many_arguments)]
    const fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
        event_application: EventApplication,
        cursor_codec: EventCursorCodec,
        ui_installations: Arc<release_postgres::ReleaseService>,
        ui_navigator: Arc<release_postgres::PgUiInstallationNavigator>,
        ui_browser: Arc<release_postgres::PgUiBrowserSessionStore>,
        ui_request_audit: Arc<dyn release_service::UiRequestAuditSink>,
        cursor_key: [u8; 32],
    ) -> Self {
        Self {
            application: ReleaseApplication::new(pool),
            event_application,
            cursor_codec,
            ui_installations,
            ui_navigator,
            ui_browser,
            ui_request_audit,
            ui_cursor_codec: ui::UiInstallationCursorCodec::new(cursor_key),
            authenticator,
            receipts,
        }
    }
}

/// Registers the generated release service.
// The registration boundary receives each independently owned adapter so the
// composition root remains explicit and testable.
#[allow(clippy::too_many_arguments)]
pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
    event_wakeups: std::sync::Arc<dyn EventWakeupSource>,
    cursor_key: [u8; 32],
    ui_installations: Arc<release_postgres::ReleaseService>,
    ui_navigator: Arc<release_postgres::PgUiInstallationNavigator>,
    ui_browser: Arc<release_postgres::PgUiBrowserSessionStore>,
    ui_request_audit: Arc<dyn release_service::UiRequestAuditSink>,
) -> Router {
    Arc::new(ReleaseRpc::new(
        pool.clone(),
        authenticator,
        receipts,
        EventApplication::new(pool, event_wakeups),
        EventCursorCodec::new(cursor_key),
        ui_installations,
        ui_navigator,
        ui_browser,
        ui_request_audit,
        cursor_key,
    ))
    .register(router)
}

#[allow(refining_impl_trait)]
impl ReleaseService for ReleaseRpc {
    async fn activate_ui(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::ActivateUiRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::ActivateUiResponse>
    {
        ui::activate(self, ctx, request).await
    }

    async fn rollback_ui(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::RollbackUiRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::RollbackUiResponse>
    {
        ui::rollback(self, ctx, request).await
    }

    async fn disable_ui(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::DisableUiRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::DisableUiResponse>
    {
        ui::disable(self, ctx, request).await
    }

    async fn remove_ui(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::RemoveUiRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::RemoveUiResponse>
    {
        ui::remove(self, ctx, request).await
    }

    async fn install_ui(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::InstallUiRequest,
        >,
    ) -> connectrpc::ServiceResult<rpc_proto::messages::hephaestus::release::v1::InstallUiResponse>
    {
        ui::install(self, ctx, request).await
    }

    async fn list_ui_installations(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::ListUiInstallationsRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::release::v1::ListUiInstallationsResponse,
    > {
        ui::list(self, ctx, request).await
    }

    async fn create_ui_browser_handoff(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::release::v1::CreateUiBrowserHandoffResponse,
    > {
        ui::handoff(self, ctx, request).await
    }

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
