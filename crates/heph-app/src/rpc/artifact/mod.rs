//! Artifact Connect service composition.

mod get_artifact_preview;
mod model;
mod stream_artifact;

use super::MediatorAuthenticator;
use crate::application::artifact::ArtifactApplication;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use release_artifact_store::LocalArtifactStore;
use rpc_proto::connect::hephaestus::artifact::v1::{ArtifactService, ArtifactServiceExt};
use sha2::{Digest, Sha256};
use std::sync::Arc;

pub(super) struct ArtifactRpc {
    application: ArtifactApplication,
    authenticator: MediatorAuthenticator,
}

impl ArtifactRpc {
    const fn new(
        pool: PgPool,
        store: LocalArtifactStore,
        cursor_key: [u8; 32],
        authenticator: MediatorAuthenticator,
    ) -> Self {
        Self {
            application: ArtifactApplication::new(pool, store, cursor_key),
            authenticator,
        }
    }
}

/// Registers the generated artifact service.
pub(super) fn register(
    router: Router,
    pool: PgPool,
    store: LocalArtifactStore,
    cursor_key: &[u8],
    authenticator: MediatorAuthenticator,
) -> Router {
    let cursor_key: [u8; 32] = Sha256::digest(cursor_key).into();
    Arc::new(ArtifactRpc::new(pool, store, cursor_key, authenticator)).register(router)
}

#[allow(refining_impl_trait)]
impl ArtifactService for ArtifactRpc {
    async fn get_artifact_preview(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::artifact::v1::GetArtifactPreviewRequest,
        >,
    ) -> connectrpc::ServiceResult<
        rpc_proto::messages::hephaestus::artifact::v1::GetArtifactPreviewResponse,
    > {
        get_artifact_preview::handle(self, ctx, request).await
    }

    async fn stream_artifact(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<
            '_,
            rpc_proto::messages::hephaestus::artifact::v1::StreamArtifactRequest,
        >,
    ) -> connectrpc::ServiceResult<
        connectrpc::ServiceStream<
            rpc_proto::messages::hephaestus::artifact::v1::StreamArtifactResponse,
        >,
    > {
        stream_artifact::handle(self, ctx, request).await
    }
}

#[cfg(test)]
mod tests {
    use connectrpc_reflection::Reflector;
    use std::sync::Arc;

    #[test]
    fn reflection_exposes_artifact_service() {
        let reflector = Reflector::from_descriptor_pool(Arc::new(
            rpc_proto::descriptor_pool().expect("checked-in descriptor pool"),
        ))
        .expect("reflection index");
        assert!(
            reflector
                .service_names()
                .iter()
                .any(|service| service == "hephaestus.artifact.v1.ArtifactService")
        );
    }
}
