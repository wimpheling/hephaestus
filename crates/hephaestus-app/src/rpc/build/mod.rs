//! Build Connect service composition.

mod get_build;
pub(super) mod model;
mod request_build;

use super::{MediatorAuthenticator, MutationReceipts};
use crate::application::build::BuildApplication;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::connect::hephaestus::build::v1::{BuildService, BuildServiceExt};
use std::sync::Arc;

pub(super) struct BuildRpc {
    application: BuildApplication,
    authenticator: MediatorAuthenticator,
    receipts: MutationReceipts,
}

impl BuildRpc {
    const fn new(
        pool: PgPool,
        authenticator: MediatorAuthenticator,
        receipts: MutationReceipts,
    ) -> Self {
        Self {
            application: BuildApplication::new(pool),
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
) -> Router {
    Arc::new(BuildRpc::new(pool, authenticator, receipts)).register(router)
}

#[allow(refining_impl_trait)]
impl BuildService for BuildRpc {
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
