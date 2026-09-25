//! Immutable OCI image catalog RPC composition.

mod get_image;
mod list_images;

use builder_catalog_application::{ImageCatalogApplication, ImageCatalogError};
use builder_catalog_domain::{
    AvailabilityState, OciImage, RegistryAvailabilityState, RegistryEvidence,
    RegistryEvidenceState, RegistryPublication as DomainRegistryPublication,
    RegistryPublicationState as DomainRegistryPublicationState,
};
use builder_catalog_postgres::PgOciImageCatalog;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::image::v1::{ImageCatalogService, ImageCatalogServiceExt},
    messages::hephaestus::{
        common::v1::OpaqueId,
        image::v1::{
            GetImageRequest, GetImageResponse, ImageAvailabilityState as RpcImageAvailabilityState,
            ImagePreparationState as RpcImagePreparationState, ListImagesRequest,
            ListImagesResponse, OciImage as RpcOciImage, Provenance,
            RegistryAvailabilityState as RpcRegistryAvailabilityState,
            RegistryEvidence as RpcRegistryEvidence,
            RegistryEvidenceState as RpcRegistryEvidenceState,
            RegistryPublication as RpcRegistryPublication,
            RegistryPublicationState as RpcRegistryPublicationState, Toolchain,
        },
    },
};
use std::sync::Arc;

pub(super) const LIST_AUDIENCE: &str = "/hephaestus.image.v1.ImageCatalogService/ListImages";
pub(super) const GET_AUDIENCE: &str = "/hephaestus.image.v1.ImageCatalogService/GetImage";
pub(super) const DEFAULT_PAGE_SIZE: u32 = 50;
pub(super) const MAX_PAGE_SIZE: u32 = 200;

pub(super) struct ImageCatalogRpc {
    pub(super) application: ImageCatalogApplication<PgOciImageCatalog>,
    pub(super) authenticator: super::MediatorAuthenticator,
}

pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: super::MediatorAuthenticator,
) -> Router {
    ImageCatalogServiceExt::register(
        Arc::new(ImageCatalogRpc {
            application: ImageCatalogApplication::new(Arc::new(PgOciImageCatalog::new(pool))),
            authenticator,
        }),
        router,
    )
}

#[allow(refining_impl_trait)]
impl ImageCatalogService for ImageCatalogRpc {
    async fn list_images(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, ListImagesRequest>,
    ) -> connectrpc::ServiceResult<ListImagesResponse> {
        list_images::handle(self, ctx, request).await
    }

    async fn get_image(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, GetImageRequest>,
    ) -> connectrpc::ServiceResult<GetImageResponse> {
        get_image::handle(self, ctx, request).await
    }
}

#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_catalog_error(error: ImageCatalogError) -> super::RpcError {
    match error {
        ImageCatalogError::NotFound => super::RpcError::NotFound,
        ImageCatalogError::Storage(_) | ImageCatalogError::InvalidData(_) => {
            super::RpcError::Unavailable
        }
        _ => super::RpcError::Unavailable,
    }
}

pub(super) fn to_proto_with_registry(
    image: OciImage,
    registry_publication: DomainRegistryPublication,
) -> RpcOciImage {
    let mut image = to_proto(image);
    image.registry_publication = to_registry_publication(registry_publication).into();
    image
}

fn to_proto(image: OciImage) -> RpcOciImage {
    RpcOciImage {
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
        preparation: RpcImagePreparationState::Ready.into(),
        availability: availability(image.availability).into(),
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

fn to_registry_publication(value: DomainRegistryPublication) -> RpcRegistryPublication {
    RpcRegistryPublication {
        state: registry_publication_state(value.state).into(),
        availability: registry_availability(value.availability).into(),
        immutable_reference: value
            .immutable_reference
            .map(|reference| reference.to_string()),
        architectures: value.architectures,
        sbom: to_registry_evidence(value.sbom).into(),
        provenance: to_registry_evidence(value.provenance).into(),
        scan: to_registry_evidence(value.scan).into(),
        signature: to_registry_evidence(value.signature).into(),
        ..Default::default()
    }
}

fn to_registry_evidence(value: RegistryEvidence) -> RpcRegistryEvidence {
    RpcRegistryEvidence {
        state: registry_evidence_state(value.state).into(),
        immutable_reference: value
            .immutable_reference
            .map(|reference| reference.to_string()),
        ..Default::default()
    }
}

const fn registry_publication_state(
    value: DomainRegistryPublicationState,
) -> RpcRegistryPublicationState {
    match value {
        DomainRegistryPublicationState::Pending => RpcRegistryPublicationState::Pending,
        DomainRegistryPublicationState::Publishing => RpcRegistryPublicationState::Publishing,
        DomainRegistryPublicationState::Verified => RpcRegistryPublicationState::Verified,
        DomainRegistryPublicationState::Approved => RpcRegistryPublicationState::Approved,
        DomainRegistryPublicationState::Missing => RpcRegistryPublicationState::Missing,
        DomainRegistryPublicationState::Retired => RpcRegistryPublicationState::Retired,
    }
}

const fn registry_availability(value: RegistryAvailabilityState) -> RpcRegistryAvailabilityState {
    match value {
        RegistryAvailabilityState::Available => RpcRegistryAvailabilityState::Available,
        RegistryAvailabilityState::Unavailable => RpcRegistryAvailabilityState::Unavailable,
        RegistryAvailabilityState::Retired => RpcRegistryAvailabilityState::Retired,
    }
}

const fn registry_evidence_state(value: RegistryEvidenceState) -> RpcRegistryEvidenceState {
    match value {
        RegistryEvidenceState::Pending => RpcRegistryEvidenceState::Pending,
        RegistryEvidenceState::Verified => RpcRegistryEvidenceState::Verified,
        RegistryEvidenceState::NotRequired => RpcRegistryEvidenceState::NotRequired,
    }
}

const fn availability(value: AvailabilityState) -> RpcImageAvailabilityState {
    match value {
        AvailabilityState::Available => RpcImageAvailabilityState::Available,
        AvailabilityState::Unavailable => RpcImageAvailabilityState::Unavailable,
        AvailabilityState::Retired => RpcImageAvailabilityState::Retired,
    }
}
