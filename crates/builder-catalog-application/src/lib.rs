//! Application port and policy validation for the builder image catalog.

use agent_config::{AgentConfig, NetworkProfile};
use async_trait::async_trait;
use builder_catalog_domain::{
    BuildNetworkPolicy, BuilderCatalogValueError, BuilderImage, BuilderImageId,
    BuilderImageReference, BuilderSelectionError, ValidatedBuilderSelection,
};
use std::sync::Arc;

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
        let reference = BuilderImageReference::parse(build.root_image.clone())
            .map_err(|_| BuilderSelectionError::UnknownImage)?;
        let image = self
            .catalog
            .find_builder_image_by_reference(&reference)
            .await
            .map_err(BuilderCatalogApplicationError::Catalog)?;
        let image = validate_entry(image).map_err(BuilderCatalogApplicationError::Catalog)?;
        let network = network_policy(build.network.profile);
        image
            .validate_selection(network, build.resources.vcpus, build.resources.memory_mib)
            .map_err(BuilderCatalogApplicationError::Selection)
    }
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use builder_catalog_domain::{
        AvailabilityState, BuilderImageId, BuilderKey, BuilderProvenance, DependencyPolicy,
        PreparationState, Toolchain,
    };

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
}
