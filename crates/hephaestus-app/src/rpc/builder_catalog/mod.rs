//! Builder-image catalog RPC composition.

mod get_builder_image;
mod list_builder_images;
mod project_builder;
mod validate_agent_config;

use crate::application::project::{ProjectApplication, ProjectError};
use builder_catalog_application::{
    BuilderCatalogApplication, BuilderCatalogApplicationError, BuilderCatalogError,
    ProjectBuilderApplication, ProjectBuilderApplicationError,
};
use builder_catalog_domain::{
    AvailabilityState, BuildNetworkPolicy, BuilderImage, BuilderSelectionError, DependencyPolicy,
    PreparationState, ProjectBuilderDefinition, ProjectBuilderProvenance, ProjectBuilderStatus,
};
use builder_catalog_postgres::PgBuilderCatalog;
use connectrpc::Router;
use control_plane_postgres::ControlPlanePool as PgPool;
use rpc_proto::{
    connect::hephaestus::builder::v1::{BuilderCatalogService, BuilderCatalogServiceExt},
    messages::hephaestus::{
        builder::v1::{
            AvailabilityState as RpcAvailabilityState, BuilderImage as RpcBuilderImage,
            CompleteProjectBuilderPreparationRequest, CompleteProjectBuilderPreparationResponse,
            CreateProjectBuilderRequest, CreateProjectBuilderResponse,
            DependencyPolicy as RpcDependencyPolicy, GetBuilderImageRequest,
            GetBuilderImageResponse, GetProjectBuilderRequest, GetProjectBuilderResponse,
            ListBuilderImagesRequest, ListBuilderImagesResponse, ListProjectBuildersRequest,
            ListProjectBuildersResponse, PreparationState as RpcPreparationState,
            ProjectBuilder as RpcProjectBuilder,
            ProjectBuilderProvenance as RpcProjectBuilderProvenance, Provenance,
            RequestProjectBuilderPreparationRequest, RequestProjectBuilderPreparationResponse,
            Toolchain, ValidateAgentConfigRequest, ValidateAgentConfigResponse,
        },
        common::v1::{NetworkPolicy, OpaqueId},
    },
};
use std::sync::Arc;
use uuid::Uuid;

const LIST_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/ListBuilderImages";
const GET_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/GetBuilderImage";
const VALIDATE_AUDIENCE: &str = "/hephaestus.builder.v1.BuilderCatalogService/ValidateAgentConfig";
const CREATE_PROJECT_BUILDER_AUDIENCE: &str =
    "/hephaestus.builder.v1.BuilderCatalogService/CreateProjectBuilder";
const LIST_PROJECT_BUILDERS_AUDIENCE: &str =
    "/hephaestus.builder.v1.BuilderCatalogService/ListProjectBuilders";
const GET_PROJECT_BUILDER_AUDIENCE: &str =
    "/hephaestus.builder.v1.BuilderCatalogService/GetProjectBuilder";
const REQUEST_PROJECT_BUILDER_PREPARATION_AUDIENCE: &str =
    "/hephaestus.builder.v1.BuilderCatalogService/RequestProjectBuilderPreparation";
const COMPLETE_PROJECT_BUILDER_PREPARATION_AUDIENCE: &str =
    "/hephaestus.builder.v1.BuilderCatalogService/CompleteProjectBuilderPreparation";
const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 200;

pub(super) struct BuilderCatalogRpc {
    pub(super) application: BuilderCatalogApplication<PgBuilderCatalog>,
    pub(super) project_application: ProjectBuilderApplication<PgBuilderCatalog>,
    pub(super) project_authorization: ProjectApplication,
    pub(super) authenticator: super::MediatorAuthenticator,
}

pub(super) fn register(
    router: Router,
    pool: PgPool,
    authenticator: super::MediatorAuthenticator,
) -> Router {
    BuilderCatalogServiceExt::register(
        Arc::new(BuilderCatalogRpc {
            application: BuilderCatalogApplication::new(Arc::new(PgBuilderCatalog::new(
                pool.clone(),
            ))),
            project_application: ProjectBuilderApplication::new(Arc::new(PgBuilderCatalog::new(
                pool.clone(),
            ))),
            project_authorization: ProjectApplication::new(pool),
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

    async fn create_project_builder(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, CreateProjectBuilderRequest>,
    ) -> connectrpc::ServiceResult<CreateProjectBuilderResponse> {
        project_builder::create(self, ctx, request).await
    }

    async fn list_project_builders(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, ListProjectBuildersRequest>,
    ) -> connectrpc::ServiceResult<ListProjectBuildersResponse> {
        project_builder::list(self, ctx, request).await
    }

    async fn get_project_builder(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, GetProjectBuilderRequest>,
    ) -> connectrpc::ServiceResult<GetProjectBuilderResponse> {
        project_builder::get(self, ctx, request).await
    }

    async fn request_project_builder_preparation(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, RequestProjectBuilderPreparationRequest>,
    ) -> connectrpc::ServiceResult<RequestProjectBuilderPreparationResponse> {
        project_builder::request_preparation(self, ctx, request).await
    }

    async fn complete_project_builder_preparation(
        &self,
        ctx: connectrpc::RequestContext,
        request: connectrpc::ServiceRequest<'_, CompleteProjectBuilderPreparationRequest>,
    ) -> connectrpc::ServiceResult<CompleteProjectBuilderPreparationResponse> {
        project_builder::complete_preparation(self, ctx, request).await
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
        BuilderCatalogApplicationError::Catalog(_)
        | BuilderCatalogApplicationError::ProjectStore(_) => super::RpcError::Unavailable,
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

pub(super) fn map_project_builder_error(error: ProjectBuilderApplicationError) -> super::RpcError {
    match error {
        ProjectBuilderApplicationError::Catalog(error) => match error {
            BuilderCatalogError::NotFound => super::RpcError::NotFound,
            BuilderCatalogError::InvalidData(_) => super::RpcError::InvalidArgument,
            _ => super::RpcError::Unavailable,
        },
        ProjectBuilderApplicationError::Store(error) => match error {
            builder_catalog_application::ProjectBuilderStoreError::NotFound => {
                super::RpcError::NotFound
            }
            builder_catalog_application::ProjectBuilderStoreError::Conflict => {
                super::RpcError::FailedPrecondition
            }
            builder_catalog_application::ProjectBuilderStoreError::InvalidData(_) => {
                super::RpcError::InvalidArgument
            }
            _ => super::RpcError::Unavailable,
        },
        ProjectBuilderApplicationError::Invalid(_) => super::RpcError::InvalidArgument,
        ProjectBuilderApplicationError::BaseImageNotApproved
        | ProjectBuilderApplicationError::Lifecycle(_) => super::RpcError::FailedPrecondition,
    }
}

// This mapper consumes the typed error so callers can pass it directly to
// `Result::map_err` without repeating a borrowing closure at every handler.
#[allow(clippy::needless_pass_by_value)]
pub(super) fn map_project_error(error: ProjectError) -> super::RpcError {
    match error {
        ProjectError::PermissionDenied => super::RpcError::PermissionDenied,
        ProjectError::NotFound => super::RpcError::NotFound,
        ProjectError::InvalidPage => super::RpcError::InvalidArgument,
        ProjectError::Persistence(_) => super::RpcError::Unavailable,
    }
}

pub(super) fn parse_uuid(value: Option<&OpaqueId>) -> Result<Uuid, super::RpcError> {
    crate::rpc::request::required_id(value)
        .and_then(|value| Uuid::parse_str(&value).map_err(|_| super::RpcError::InvalidArgument))
}

pub(super) async fn authorize_project(
    service: &BuilderCatalogRpc,
    identity: &identity_domain::AuthenticatedIdentity,
    project_id: Uuid,
) -> Result<(), super::RpcError> {
    service
        .project_authorization
        .get(identity, project_id)
        .await
        .map(|_| ())
        .map_err(map_project_error)
}

pub(super) fn to_project_builder(value: ProjectBuilderDefinition) -> RpcProjectBuilder {
    RpcProjectBuilder {
        id: opaque(value.id.as_uuid()).into(),
        project_id: opaque(value.project_id).into(),
        source_repository_id: opaque(value.source_repository_id).into(),
        key: value.key.to_string(),
        display_name: value.display_name,
        source_revision: value.source_revision,
        dockerfile_path: value.dockerfile_path.to_string(),
        context_path: value.context_path.to_string(),
        context_digest: value.context_digest.to_string(),
        approved_base_image_reference: value.approved_base_image.to_string(),
        status: project_builder_status(value.status).into(),
        oci_image_reference: value.oci_image_reference.map(|value| value.to_string()),
        oci_image_digest: value.oci_image_digest.map(|value| value.to_string()),
        provenance: value.provenance.map(to_project_provenance).into(),
        failure_reason: value.failure_reason,
        created_at: timestamp(value.created_at).into(),
        updated_at: timestamp(value.updated_at).into(),
        ..Default::default()
    }
}

fn to_project_provenance(value: ProjectBuilderProvenance) -> RpcProjectBuilderProvenance {
    RpcProjectBuilderProvenance {
        source_revision: value.source_revision,
        context_digest: value.context_digest.to_string(),
        attestation_reference: value.attestation_reference,
        sbom_reference: value.sbom_reference,
        ..Default::default()
    }
}

const fn project_builder_status(
    value: ProjectBuilderStatus,
) -> rpc_proto::messages::hephaestus::builder::v1::ProjectBuilderStatus {
    use rpc_proto::messages::hephaestus::builder::v1::ProjectBuilderStatus as Status;
    match value {
        ProjectBuilderStatus::Draft => Status::Draft,
        ProjectBuilderStatus::Preparing => Status::Preparing,
        ProjectBuilderStatus::Ready => Status::Ready,
        ProjectBuilderStatus::Failed => Status::Failed,
        ProjectBuilderStatus::Retired => Status::Retired,
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn timestamp(value: time::OffsetDateTime) -> buffa_types::google::protobuf::Timestamp {
    buffa_types::google::protobuf::Timestamp {
        seconds: value.unix_timestamp(),
        nanos: value.nanosecond().cast_signed(),
        ..Default::default()
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

#[cfg(test)]
mod tests {
    use super::{map_project_builder_error, parse_uuid};
    use builder_catalog_application::{
        BuilderCatalogError, ProjectBuilderApplicationError, ProjectBuilderStoreError,
    };
    use rpc_proto::messages::hephaestus::common::v1::OpaqueId;
    use uuid::Uuid;

    #[test]
    fn project_builder_errors_preserve_resource_and_lifecycle_categories() {
        assert_eq!(
            map_project_builder_error(ProjectBuilderApplicationError::Catalog(
                BuilderCatalogError::NotFound,
            )),
            crate::rpc::RpcError::NotFound
        );
        assert_eq!(
            map_project_builder_error(ProjectBuilderApplicationError::Store(
                ProjectBuilderStoreError::Conflict,
            )),
            crate::rpc::RpcError::FailedPrecondition
        );
    }

    #[test]
    fn project_builder_ids_require_uuid_opaque_ids() {
        let id = Uuid::new_v4();
        assert_eq!(
            parse_uuid(Some(&OpaqueId {
                value: id.to_string(),
                ..Default::default()
            })),
            Ok(id)
        );
        assert!(
            parse_uuid(Some(&OpaqueId {
                value: String::from("not-an-id"),
                ..Default::default()
            }))
            .is_err()
        );
    }
}
