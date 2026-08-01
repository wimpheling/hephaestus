//! Builder-image catalog RPC composition.

mod get_builder_image;
mod list_builder_images;
mod validate_agent_config;

use builder_catalog_application::{
    BuilderCatalogApplication, BuilderCatalogApplicationError, BuilderCatalogError,
};
use builder_catalog_domain::{
    AvailabilityState, BuildNetworkPolicy, BuilderImage, BuilderSelectionError, DependencyPolicy,
    PreparationState,
};
use builder_catalog_postgres::PgBuilderCatalog;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::builder::v1::{BuilderCatalogService, BuilderCatalogServiceExt},
    messages::hephaestus::{
        builder::v1::{
            AvailabilityState as RpcAvailabilityState, BuilderImage as RpcBuilderImage,
            DependencyPolicy as RpcDependencyPolicy, GetBuilderImageRequest,
            GetBuilderImageResponse, ListBuilderImagesRequest, ListBuilderImagesResponse,
            PreparationState as RpcPreparationState, Provenance, Toolchain,
            ValidateAgentConfigRequest, ValidateAgentConfigResponse,
        },
        common::v1::{NetworkPolicy, OpaqueId},
    },
};
use std::sync::Arc;

const LIST_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/ListBuilderImages";
const GET_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/GetBuilderImage";
const VALIDATE_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/ValidateAgentConfig";
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub(super) struct BuilderCatalogRpc {
    pub(super) application: BuilderCatalogApplication<PgBuilderCatalog>,
    pub(super) authenticator: super::MediatorAuthenticator,
}

pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: super::MediatorAuthenticator,
) -> Router {
    BuilderCatalogServiceExt::register(
        Arc::new(BuilderCatalogRpc {
            application: BuilderCatalogApplication::new(Arc::new(PgBuilderCatalog::new(pool))),
            authenticator,
        }),
        router,
    )
}

#[allow(refining_impl_trait)]
impl BuilderCatalogService for BuilderCatalogRpc {
    async fn list_builder_images(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, ListBuilderImagesRequest>,
    ) -> connectrpc::ServiceResult<ListBuilderImagesResponse> {
        list_builder_images::handle(self, ctx, request).await
    }

    async fn get_builder_image(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, GetBuilderImageRequest>,
    ) -> connectrpc::ServiceResult<GetBuilderImageResponse> {
        get_builder_image::handle(self, ctx, request).await
    }

    async fn validate_agent_config(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, ValidateAgentConfigRequest>,
    ) -> connectrpc::ServiceResult<ValidateAgentConfigResponse> {
        validate_agent_config::handle(self, ctx, request).await
    }
}

// These mappers consume the typed errors passed by `Result::map_err`; a
// borrowing version would require otherwise needless closures at every RPC.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_error(error: BuilderCatalogApplicationError) -> super::RpcError {
    match error {
        BuilderCatalogApplicationError::Catalog(BuilderCatalogError::NotFound)
        | BuilderCatalogApplicationError::Selection(BuilderSelectionError::UnknownImage) => {
            super::RpcError::NotFound
        }
        BuilderCatalogApplicationError::Selection(_) => super::RpcError::InvalidArgument,
        BuilderCatalogApplicationError::Catalog(_) => super::RpcError::Unavailable,
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_catalog_error(error: BuilderCatalogError) -> super::RpcError {
    match error {
        BuilderCatalogError::NotFound => super::RpcError::NotFound,
        BuilderCatalogError::Storage(_) | BuilderCatalogError::InvalidData(_) => {
            super::RpcError::Unavailable
        }
        _ => super::RpcError::Unavailable,
    }
}

pub(super) fn to_proto(image: BuilderImage) -> RpcBuilderImage {
    RpcBuilderImage {
        id: OpaqueId {
            value: image.id.to_string(),
            ..Default::default()
        }
        .into(),
        key: image.key.to_string(),
        display_name: image.display_name,
        image_reference: image.image_reference.to_string(),
        toolchains: image
            .toolchains
            .into_iter()
            .map(|toolchain| Toolchain {
                name: toolchain.name,
                version: toolchain.version,
                ..Default::default()
            })
            .collect(),
        architectures: image.architectures,
        preparation: preparation(image.preparation).into(),
        availability: availability(image.availability).into(),
        network_ceiling: network(image.network_ceiling).into(),
        max_vcpus: u32::from(image.max_vcpus),
        max_memory_mib: image.max_memory_mib,
        dependency_policy: dependency_policy(image.dependency_policy).into(),
        provenance: Provenance {
            source: image.provenance.source,
            signature: image.provenance.signature,
            sbom: image.provenance.sbom,
            ..Default::default()
        }
        .into(),
        platform_policy_version: image.platform_policy_version,
        ..Default::default()
    }
}

const fn preparation(value: PreparationState) -> RpcPreparationState {
    match value {
        PreparationState::Ready => RpcPreparationState::Ready,
        PreparationState::Preparing => RpcPreparationState::Preparing,
        PreparationState::Failed => RpcPreparationState::Failed,
    }
}

const fn availability(value: AvailabilityState) -> RpcAvailabilityState {
    match value {
        AvailabilityState::Available => RpcAvailabilityState::Available,
        AvailabilityState::Unavailable => RpcAvailabilityState::Unavailable,
        AvailabilityState::Retired => RpcAvailabilityState::Retired,
    }
}

const fn dependency_policy(value: DependencyPolicy) -> RpcDependencyPolicy {
    match value {
        DependencyPolicy::VendoredOffline => RpcDependencyPolicy::VendoredOffline,
        DependencyPolicy::ReadOnlyPlatformCache => RpcDependencyPolicy::ReadOnlyPlatformCache,
        DependencyPolicy::ConstrainedRegistryEgress => {
            RpcDependencyPolicy::ConstrainedRegistryEgress
        }
    }
}

const fn network(value: BuildNetworkPolicy) -> NetworkPolicy {
    match value {
        BuildNetworkPolicy::Disabled => NetworkPolicy::Disabled,
        BuildNetworkPolicy::BrokerOnly => NetworkPolicy::BrokerOnly,
        BuildNetworkPolicy::Egress => NetworkPolicy::Egress,
    }
}
