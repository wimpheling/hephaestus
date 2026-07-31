//! Release Connect service composition.

mod get_release;
mod list_repository_releases;
mod model;

use super::MediatorAuthenticator;
use crate::application::release::ReleaseApplication;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::connect::hephaestus::release::v1::{ReleaseService, ReleaseServiceExt};
use std::sync::Arc;

pub(super) struct ReleaseRpc {
    application: ReleaseApplication,
    authenticator: MediatorAuthenticator,
}

impl ReleaseRpc {
    const fn new(pool: PgPool, authenticator: MediatorAuthenticator) -> Self {
        Self {
            application: ReleaseApplication::new(pool),
            authenticator,
        }
    }
}

/// Registers the generated release service.
pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: MediatorAuthenticator,
) -> Router {
    Arc::new(ReleaseRpc::new(pool, authenticator)).register(router)
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
