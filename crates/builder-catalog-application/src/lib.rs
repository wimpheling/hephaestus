//! Application port and policy validation for the builder image catalog.

use agent_config::{AgentConfig, NetworkProfile};
use async_trait::async_trait;
use builder_catalog_domain::{
    BuildNetworkPolicy, BuilderCatalogValueError, BuilderImage, BuilderImageId,
    BuilderImageReference, BuilderSelectionError, NewProjectBuilder, OciDigest,
    ProjectBuilderDefinition, ProjectBuilderId, ProjectBuilderLifecycleError,
    ProjectBuilderProvenance, ProjectBuilderStatus, ValidatedBuilderSelection,
};
use std::sync::Arc;
use uuid::Uuid;

/// Provider-neutral builder catalog persistence failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuilderCatalogError {
    /// The requested image does not exist.
    #[error("builder image was not found")]
    NotFound,
    /// Persistence or transport failed.
    #[error("builder catalog storage failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// Persisted metadata violates the domain contract.
    #[error("builder catalog contains invalid metadata: {0}")]
    InvalidData(#[source] BuilderCatalogValueError),
}

/// Persistence boundary for platform-owned builder images.
#[async_trait]
pub trait BuilderCatalog: Send + Sync + 'static {
    /// Lists catalog entries in stable platform order.
    async fn list_builder_images(&self) -> Result<Vec<BuilderImage>, BuilderCatalogError>;

    /// Loads one entry by stable identity.
    async fn get_builder_image(
        &self,
        id: BuilderImageId,
    ) -> Result<BuilderImage, BuilderCatalogError>;

    /// Loads one entry by its exact digest-pinned reference.
    async fn find_builder_image_by_reference(
        &self,
        reference: &BuilderImageReference,
    ) -> Result<BuilderImage, BuilderCatalogError>;

    /// Loads a catalog entry by its stable platform key.
    async fn find_builder_image_by_key(
        &self,
        key: &str,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        self.list_builder_images()
            .await?
            .into_iter()
            .find(|image| image.key.as_str() == key)
            .ok_or(BuilderCatalogError::NotFound)
    }
}

/// Application service exposing catalog reads and source-selection validation.
pub struct BuilderCatalogApplication<C> {
    catalog: Arc<C>,
}

impl<C> BuilderCatalogApplication<C>
where
    C: BuilderCatalog,
{
    /// Creates the application service over one catalog adapter.
    #[must_use]
    pub const fn new(catalog: Arc<C>) -> Self {
        Self { catalog }
    }

    /// Lists validated platform-owned catalog entries.
    ///
    /// # Errors
    ///
    /// Returns a persistence or invalid-data failure.
    pub async fn list_builder_images(&self) -> Result<Vec<BuilderImage>, BuilderCatalogError> {
        let entries = self.catalog.list_builder_images().await?;
        entries
            .into_iter()
            .map(validate_entry)
            .collect::<Result<Vec<_>, _>>()
    }

    /// Loads and validates one catalog entry.
    ///
    /// # Errors
    ///
    /// Returns a persistence or invalid-data failure.
    pub async fn get_builder_image(
        &self,
        id: BuilderImageId,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        validate_entry(self.catalog.get_builder_image(id).await?)
    }

    /// Resolves and validates the build section of a parsed `agent.toml`.
    ///
    /// The exact digest-pinned reference in `build.root_image` is the only
    /// accepted selection identity. No environment variable or arbitrary image
    /// lookup is consulted.
    ///
    /// # Errors
    ///
    /// Returns a stable selection failure or catalog persistence failure.
    pub async fn validate_agent_config(
        &self,
        config: &AgentConfig,
    ) -> Result<ValidatedBuilderSelection, BuilderCatalogApplicationError> {
        let build = config
            .build
            .as_ref()
            .ok_or(BuilderSelectionError::MissingBuild)?;
        self.validate_build_config(build).await
    }

    /// Resolves and validates an already parsed isolated build definition.
    ///
    /// # Errors
    ///
    /// Returns a stable selection failure or catalog persistence failure.
    pub async fn validate_build_config(
        &self,
        build: &agent_config::BuildConfig,
    ) -> Result<ValidatedBuilderSelection, BuilderCatalogApplicationError> {
        let (image, selected_key, selected_reference) = match (&build.root_image, &build.builder) {
            (Some(reference), None) => {
                let reference = BuilderImageReference::parse(reference.clone())
                    .map_err(|_| BuilderSelectionError::UnknownImage)?;
                let image = self
                    .catalog
                    .find_builder_image_by_reference(&reference)
                    .await
                    .map_err(BuilderCatalogApplicationError::Catalog)?;
                (image, None, reference)
            }
            (None, Some(agent_config::BuilderSelection::Platform { key })) => {
                let image = self
                    .catalog
                    .find_builder_image_by_key(key)
                    .await
                    .map_err(BuilderCatalogApplicationError::Catalog)?;
                let reference = image.image_reference.clone();
                (image, Some(key.clone()), reference)
            }
            _ => return Err(BuilderSelectionError::UnknownImage.into()),
        };
        validate_selection(image, selected_key, selected_reference, build)
    }
}

impl<C> BuilderCatalogApplication<C>
where
    C: BuilderCatalog + ProjectBuilderStore,
{
    /// Resolves a config that may select a project-owned prepared builder.
    ///
    /// # Errors
    ///
    /// Returns a selection, catalog, or project-builder persistence failure.
    pub async fn validate_agent_config_for_project(
        &self,
        config: &AgentConfig,
        project_id: Uuid,
    ) -> Result<ValidatedBuilderSelection, BuilderCatalogApplicationError> {
        let build = config
            .build
            .as_ref()
            .ok_or(BuilderSelectionError::MissingBuild)?;
        self.validate_build_config_for_project(build, project_id)
            .await
    }

    async fn validate_build_config_for_project(
        &self,
        build: &agent_config::BuildConfig,
        project_id: Uuid,
    ) -> Result<ValidatedBuilderSelection, BuilderCatalogApplicationError> {
        if !matches!(
            (&build.root_image, &build.builder),
            (None, Some(agent_config::BuilderSelection::Project { .. }))
        ) {
            return self.validate_build_config(build).await;
        }
        let agent_config::BuilderSelection::Project { id } = build
            .builder
            .as_ref()
            .expect("project builder selector checked above")
        else {
            unreachable!("project builder selector checked above");
        };
        let builder_id = uuid::Uuid::parse_str(id)
            .map(builder_catalog_domain::ProjectBuilderId::from_uuid)
            .map_err(|_| BuilderSelectionError::UnknownImage)?;
        let builder = self
            .catalog
            .get_project_builder(project_id, builder_id)
            .await
            .map_err(BuilderCatalogApplicationError::ProjectStore)?;
        let output = builder
            .status
            .eq(&ProjectBuilderStatus::Ready)
            .then(|| builder.oci_image_reference.clone())
            .flatten()
            .ok_or(BuilderSelectionError::NotPrepared)?;
        let base = self
            .catalog
            .find_builder_image_by_reference(&builder.approved_base_image)
            .await
            .map_err(BuilderCatalogApplicationError::Catalog)?;
        validate_selection(base, Some(builder.key.to_string()), output, build)
    }
}

fn validate_selection(
    image: BuilderImage,
    selected_key: Option<String>,
    selected_reference: BuilderImageReference,
    build: &agent_config::BuildConfig,
) -> Result<ValidatedBuilderSelection, BuilderCatalogApplicationError> {
    let network = network_policy(build.network.profile);
    let image = validate_entry(image).map_err(BuilderCatalogApplicationError::Catalog)?;
    let mut selection = image
        .validate_selection(network, build.resources.vcpus, build.resources.memory_mib)
        .map_err(BuilderCatalogApplicationError::Selection)?;
    selection.image_reference = selected_reference;
    if let Some(key) = selected_key {
        selection.key = builder_catalog_domain::BuilderKey::parse(key).map_err(|error| {
            BuilderCatalogApplicationError::Catalog(BuilderCatalogError::InvalidData(error))
        })?;
    }
    Ok(selection)
}

fn validate_entry(entry: BuilderImage) -> Result<BuilderImage, BuilderCatalogError> {
    entry
        .validate()
        .map(|()| entry)
        .map_err(BuilderCatalogError::InvalidData)
}

const fn network_policy(profile: NetworkProfile) -> BuildNetworkPolicy {
    match profile {
        NetworkProfile::Disabled => BuildNetworkPolicy::Disabled,
        NetworkProfile::BrokerOnly => BuildNetworkPolicy::BrokerOnly,
        NetworkProfile::Egress => BuildNetworkPolicy::Egress,
    }
}

/// Application-level builder catalog failure.
#[derive(Debug, thiserror::Error)]
pub enum BuilderCatalogApplicationError {
    /// Catalog persistence failed.
    #[error(transparent)]
    Catalog(#[from] BuilderCatalogError),
    /// Source configuration cannot select the image.
    #[error(transparent)]
    Selection(#[from] BuilderSelectionError),
    /// Project-owned builder lookup failed.
    #[error(transparent)]
    ProjectStore(#[from] ProjectBuilderStoreError),
}

/// Durable project-owned builder persistence failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ProjectBuilderStoreError {
    /// The requested project builder does not exist.
    #[error("project builder was not found")]
    NotFound,
    /// The requested lifecycle operation lost a concurrent state race.
    #[error("project builder lifecycle state changed concurrently")]
    Conflict,
    /// Persisted metadata violates the project-builder domain contract.
    #[error("project builder contains invalid metadata: {0}")]
    InvalidData(#[source] BuilderCatalogValueError),
    /// Persistence or transport failed.
    #[error("project builder storage failed: {0}")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Persistence boundary for project-owned OCI builder definitions.
#[async_trait]
pub trait ProjectBuilderStore: Send + Sync + 'static {
    /// Persists a new draft definition.
    async fn create_project_builder(
        &self,
        builder: NewProjectBuilder,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;

    /// Lists definitions visible within one project.
    async fn list_project_builders(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<ProjectBuilderDefinition>, ProjectBuilderStoreError>;

    /// Loads one definition within its owning project.
    async fn get_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;

    /// Atomically moves a draft or failed definition into preparation.
    async fn begin_project_builder_preparation(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;

    /// Atomically records an immutable OCI output and its provenance.
    async fn complete_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        output_reference: BuilderImageReference,
        output_digest: OciDigest,
        provenance: ProjectBuilderProvenance,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;

    /// Atomically records a failed preparation.
    async fn fail_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        reason: String,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;

    /// Retires a definition without deleting its history.
    async fn retire_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError>;
}

/// User-supplied project builder source and policy metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectBuilderRequest {
    /// Owning project identity.
    pub project_id: Uuid,
    /// Repository containing the Dockerfile and context.
    pub source_repository_id: Uuid,
    /// Project-local builder key.
    pub key: String,
    /// Human-readable builder name.
    pub display_name: String,
    /// Immutable source commit digest.
    pub source_revision: String,
    /// Repository-relative Dockerfile path.
    pub dockerfile_path: String,
    /// Repository-relative build context path.
    pub context_path: String,
    /// Immutable digest of the submitted context.
    pub context_digest: String,
    /// Exact digest-pinned approved platform base image.
    pub approved_base_image: String,
}

/// Application service for validating and advancing project-owned builders.
pub struct ProjectBuilderApplication<C> {
    catalog: Arc<C>,
}

impl<C> ProjectBuilderApplication<C>
where
    C: BuilderCatalog + ProjectBuilderStore,
{
    /// Creates the application service over one catalog/store adapter.
    #[must_use]
    pub const fn new(catalog: Arc<C>) -> Self {
        Self { catalog }
    }

    /// Validates and persists a project-owned builder draft.
    ///
    /// The base image must be an available, ready platform catalog entry. No
    /// project-provided image reference can satisfy this policy.
    ///
    /// # Errors
    ///
    /// Returns a validation, approval, or persistence failure.
    pub async fn create_project_builder(
        &self,
        request: CreateProjectBuilderRequest,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let builder = NewProjectBuilder {
            id: ProjectBuilderId::new(),
            project_id: request.project_id,
            key: builder_catalog_domain::BuilderKey::parse(request.key)
                .map_err(ProjectBuilderApplicationError::Invalid)?,
            display_name: request.display_name,
            source_repository_id: request.source_repository_id,
            source_revision: request.source_revision,
            dockerfile_path: builder_catalog_domain::BuilderSourcePath::parse(
                request.dockerfile_path,
            )
            .map_err(ProjectBuilderApplicationError::Invalid)?,
            context_path: builder_catalog_domain::BuilderSourcePath::parse(request.context_path)
                .map_err(ProjectBuilderApplicationError::Invalid)?,
            context_digest: OciDigest::parse(request.context_digest)
                .map_err(ProjectBuilderApplicationError::Invalid)?,
            approved_base_image: BuilderImageReference::parse(request.approved_base_image)
                .map_err(ProjectBuilderApplicationError::Invalid)?,
        };
        builder
            .validate()
            .map_err(ProjectBuilderApplicationError::Invalid)?;
        self.ensure_approved_base(&builder.approved_base_image)
            .await?;
        self.catalog
            .create_project_builder(builder)
            .await
            .map_err(ProjectBuilderApplicationError::Store)
    }

    /// Lists validated project-owned builders for one project.
    ///
    /// # Errors
    ///
    /// Returns a persistence or invalid-data failure.
    pub async fn list_project_builders(
        &self,
        project_id: Uuid,
    ) -> Result<Vec<ProjectBuilderDefinition>, ProjectBuilderApplicationError> {
        let builders = self
            .catalog
            .list_project_builders(project_id)
            .await
            .map_err(ProjectBuilderApplicationError::Store)?;
        builders
            .into_iter()
            .map(|builder| {
                builder
                    .validate()
                    .map(|()| builder)
                    .map_err(ProjectBuilderApplicationError::Invalid)
            })
            .collect()
    }

    /// Loads one validated project-owned builder.
    ///
    /// # Errors
    ///
    /// Returns a persistence or invalid-data failure.
    pub async fn get_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let builder = self
            .catalog
            .get_project_builder(project_id, id)
            .await
            .map_err(ProjectBuilderApplicationError::Store)?;
        builder
            .validate()
            .map(|()| builder)
            .map_err(ProjectBuilderApplicationError::Invalid)
    }

    /// Begins a preparation attempt after validating the current state.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle or persistence failure.
    pub async fn begin_project_builder_preparation(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let current = self.get_project_builder(project_id, id).await?;
        self.ensure_approved_base(&current.approved_base_image)
            .await?;
        current
            .begin_preparation()
            .map_err(ProjectBuilderApplicationError::Lifecycle)?;
        self.catalog
            .begin_project_builder_preparation(project_id, id)
            .await
            .map_err(ProjectBuilderApplicationError::Store)
    }

    /// Completes preparation with a validated immutable OCI output.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle, validation, or persistence failure.
    pub async fn complete_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        output_reference: String,
        output_digest: String,
        provenance: ProjectBuilderProvenance,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let current = self.get_project_builder(project_id, id).await?;
        let output_reference = BuilderImageReference::parse(output_reference)
            .map_err(ProjectBuilderApplicationError::Invalid)?;
        let output_digest =
            OciDigest::parse(output_digest).map_err(ProjectBuilderApplicationError::Invalid)?;
        current
            .clone()
            .complete(
                output_reference.clone(),
                output_digest.clone(),
                provenance.clone(),
            )
            .map_err(ProjectBuilderApplicationError::Lifecycle)?;
        self.catalog
            .complete_project_builder(project_id, id, output_reference, output_digest, provenance)
            .await
            .map_err(ProjectBuilderApplicationError::Store)
    }

    /// Records a validated preparation failure.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle or persistence failure.
    pub async fn fail_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
        reason: String,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let current = self.get_project_builder(project_id, id).await?;
        current
            .clone()
            .fail(reason.clone())
            .map_err(ProjectBuilderApplicationError::Lifecycle)?;
        self.catalog
            .fail_project_builder(project_id, id, reason)
            .await
            .map_err(ProjectBuilderApplicationError::Store)
    }

    /// Retires a project builder without deleting its record.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle or persistence failure.
    pub async fn retire_project_builder(
        &self,
        project_id: Uuid,
        id: ProjectBuilderId,
    ) -> Result<ProjectBuilderDefinition, ProjectBuilderApplicationError> {
        let current = self.get_project_builder(project_id, id).await?;
        current
            .retire()
            .map_err(ProjectBuilderApplicationError::Lifecycle)?;
        self.catalog
            .retire_project_builder(project_id, id)
            .await
            .map_err(ProjectBuilderApplicationError::Store)
    }

    async fn ensure_approved_base(
        &self,
        reference: &BuilderImageReference,
    ) -> Result<(), ProjectBuilderApplicationError> {
        let image = self
            .catalog
            .find_builder_image_by_reference(reference)
            .await
            .map_err(ProjectBuilderApplicationError::Catalog)?;
        image.validate().map_err(|error| {
            ProjectBuilderApplicationError::Catalog(BuilderCatalogError::InvalidData(error))
        })?;
        if image.preparation != builder_catalog_domain::PreparationState::Ready
            || image.availability != builder_catalog_domain::AvailabilityState::Available
        {
            return Err(ProjectBuilderApplicationError::BaseImageNotApproved);
        }
        Ok(())
    }
}

/// Application-level project builder failure.
#[derive(Debug, thiserror::Error)]
pub enum ProjectBuilderApplicationError {
    /// Platform catalog lookup failed.
    #[error(transparent)]
    Catalog(#[from] BuilderCatalogError),
    /// Project builder persistence failed.
    #[error(transparent)]
    Store(#[from] ProjectBuilderStoreError),
    /// User-supplied metadata is invalid.
    #[error(transparent)]
    Invalid(#[from] BuilderCatalogValueError),
    /// The selected base is not a ready, available platform image.
    #[error("project builder base image is not an approved available platform image")]
    BaseImageNotApproved,
    /// The requested lifecycle operation is invalid.
    #[error(transparent)]
    Lifecycle(#[from] ProjectBuilderLifecycleError),
}

#[cfg(test)]
mod tests {
    use super::*;
    use builder_catalog_domain::{
        AvailabilityState, BuilderImageId, BuilderImageReference, BuilderKey, BuilderProvenance,
        DependencyPolicy, NewProjectBuilder, OciDigest, PreparationState, ProjectBuilderDefinition,
        ProjectBuilderId, ProjectBuilderProvenance, ProjectBuilderStatus, Toolchain,
    };
    use uuid::Uuid;

    struct FixtureCatalog {
        image: BuilderImage,
    }

    #[async_trait]
    impl BuilderCatalog for FixtureCatalog {
        async fn list_builder_images(&self) -> Result<Vec<BuilderImage>, BuilderCatalogError> {
            Ok(vec![self.image.clone()])
        }

        async fn get_builder_image(
            &self,
            id: BuilderImageId,
        ) -> Result<BuilderImage, BuilderCatalogError> {
            (id == self.image.id)
                .then_some(self.image.clone())
                .ok_or(BuilderCatalogError::NotFound)
        }

        async fn find_builder_image_by_reference(
            &self,
            reference: &BuilderImageReference,
        ) -> Result<BuilderImage, BuilderCatalogError> {
            (reference == &self.image.image_reference)
                .then_some(self.image.clone())
                .ok_or(BuilderCatalogError::NotFound)
        }
    }

    #[async_trait]
    impl ProjectBuilderStore for FixtureCatalog {
        async fn create_project_builder(
            &self,
            builder: NewProjectBuilder,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            let now = time::OffsetDateTime::UNIX_EPOCH;
            Ok(ProjectBuilderDefinition {
                id: builder.id,
                project_id: builder.project_id,
                key: builder.key,
                display_name: builder.display_name,
                source_repository_id: builder.source_repository_id,
                source_revision: builder.source_revision,
                dockerfile_path: builder.dockerfile_path,
                context_path: builder.context_path,
                context_digest: builder.context_digest,
                approved_base_image: builder.approved_base_image,
                status: ProjectBuilderStatus::Draft,
                oci_image_reference: None,
                oci_image_digest: None,
                provenance: None,
                failure_reason: None,
                created_at: now,
                updated_at: now,
            })
        }

        async fn list_project_builders(
            &self,
            _project_id: Uuid,
        ) -> Result<Vec<ProjectBuilderDefinition>, ProjectBuilderStoreError> {
            Err(unused_store())
        }

        async fn get_project_builder(
            &self,
            _project_id: Uuid,
            _id: ProjectBuilderId,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            Err(unused_store())
        }

        async fn begin_project_builder_preparation(
            &self,
            _project_id: Uuid,
            _id: ProjectBuilderId,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            Err(unused_store())
        }

        async fn complete_project_builder(
            &self,
            _project_id: Uuid,
            _id: ProjectBuilderId,
            _output_reference: BuilderImageReference,
            _output_digest: OciDigest,
            _provenance: ProjectBuilderProvenance,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            Err(unused_store())
        }

        async fn fail_project_builder(
            &self,
            _project_id: Uuid,
            _id: ProjectBuilderId,
            _reason: String,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            Err(unused_store())
        }

        async fn retire_project_builder(
            &self,
            _project_id: Uuid,
            _id: ProjectBuilderId,
        ) -> Result<ProjectBuilderDefinition, ProjectBuilderStoreError> {
            Err(unused_store())
        }
    }

    fn unused_store() -> ProjectBuilderStoreError {
        ProjectBuilderStoreError::Storage(Box::new(std::io::Error::other("unused fixture path")))
    }

    fn image(network_ceiling: BuildNetworkPolicy) -> BuilderImage {
        BuilderImage {
            id: BuilderImageId::new(),
            key: BuilderKey::parse("rust").expect("key"),
            display_name: String::from("Rust builder"),
            image_reference: BuilderImageReference::parse(
                "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            )
            .expect("reference"),
            toolchains: vec![Toolchain {
                name: String::from("rust"),
                version: String::from("1.88.0"),
            }],
            architectures: vec![String::from("x86_64")],
            preparation: PreparationState::Ready,
            availability: AvailabilityState::Available,
            network_ceiling,
            max_vcpus: 2,
            max_memory_mib: 512,
            dependency_policy: DependencyPolicy::VendoredOffline,
            provenance: BuilderProvenance {
                source: String::from("attestation://rust"),
                signature: None,
                sbom: None,
            },
            platform_policy_version: String::from("build/v1"),
        }
    }

    fn config(reference: &str, network: &str) -> AgentConfig {
        let source = format!(
            r#"
version = 2
[agent]
name = "builder"
key = "builder"
[build]
command = "/usr/bin/build"
working_directory = "/workspace/source"
root_image = "{reference}"
[build.resources]
vcpus = 1
memory_mib = 128
[build.network]
profile = "{network}"
[[build.artifacts]]
path = "bin/agent"
kind = "executable"
[guest]
command = "bin/agent"
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "registry.example/agent@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
[workspace]
mount = true
path = "/workspace/repo"
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
"#
        );
        agent_config::parse(source.as_bytes())
            .config
            .expect("fixture config is valid")
    }

    #[tokio::test]
    async fn validates_exact_digest_selection_and_policy() {
        let catalog = Arc::new(FixtureCatalog {
            image: image(BuildNetworkPolicy::Disabled),
        });
        let service = BuilderCatalogApplication::new(catalog);
        let selected = service
            .validate_agent_config(&config(
                "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "disabled",
            ))
            .await
            .expect("selection is valid");
        assert_eq!(selected.key.as_str(), "rust");
        assert_eq!(selected.network, BuildNetworkPolicy::Disabled);
    }

    #[tokio::test]
    async fn rejects_unknown_reference_and_broader_network() {
        let catalog = Arc::new(FixtureCatalog {
            image: image(BuildNetworkPolicy::Disabled),
        });
        let service = BuilderCatalogApplication::new(catalog);
        assert!(matches!(
            service
                .validate_agent_config(&config(
                    "registry.example/other@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                    "disabled",
                ))
                .await,
            Err(BuilderCatalogApplicationError::Catalog(
                BuilderCatalogError::NotFound
            ))
        ));
        assert!(matches!(
            service
                .validate_agent_config(&config(
                    "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "egress",
                ))
                .await,
            Err(BuilderCatalogApplicationError::Selection(
                BuilderSelectionError::NetworkCeilingExceeded
            ))
        ));
    }

    #[tokio::test]
    async fn project_builder_requires_an_available_platform_base() {
        let catalog = Arc::new(FixtureCatalog {
            image: image(BuildNetworkPolicy::Disabled),
        });
        let service = ProjectBuilderApplication::new(catalog);
        let request = CreateProjectBuilderRequest {
            project_id: Uuid::new_v4(),
            source_repository_id: Uuid::new_v4(),
            key: String::from("custom-node"),
            display_name: String::from("Custom Node"),
            source_revision: String::from("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            dockerfile_path: String::from("Dockerfile.builder"),
            context_path: String::from("."),
            context_digest: String::from(
                "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            ),
            approved_base_image: String::from(
                "registry.example/rust@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        };
        let created = service
            .create_project_builder(request.clone())
            .await
            .expect("approved platform base");
        assert_eq!(created.status, ProjectBuilderStatus::Draft);
        assert_eq!(created.context_path.as_str(), ".");

        let mut unapproved = request;
        unapproved.approved_base_image = String::from(
            "registry.example/custom@sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        );
        assert!(matches!(
            service.create_project_builder(unapproved).await,
            Err(ProjectBuilderApplicationError::Catalog(
                BuilderCatalogError::NotFound
            ))
        ));
    }
}
