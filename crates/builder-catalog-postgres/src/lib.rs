//! `PostgreSQL` adapter for the platform-owned builder image catalog.

use async_trait::async_trait;
use builder_catalog_application::{BuilderCatalog, BuilderCatalogError};
use builder_catalog_domain::{
    AvailabilityState, BuildNetworkPolicy, BuilderCatalogValueError, BuilderImage, BuilderImageId,
    BuilderImageReference, BuilderKey, BuilderProvenance, DependencyPolicy, PreparationState,
    Toolchain,
};
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

/// PostgreSQL-backed builder catalog.
#[derive(Clone)]
pub struct PgBuilderCatalog {
    pool: PgPool,
}

impl PgBuilderCatalog {
    /// Creates an adapter over the application database pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl BuilderCatalog for PgBuilderCatalog {
    async fn list_builder_images(&self) -> Result<Vec<BuilderImage>, BuilderCatalogError> {
        let rows = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images
             ORDER BY key, id",
        )
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(BuilderImageRow::try_into_domain)
            .collect()
    }

    async fn get_builder_image(
        &self,
        id: BuilderImageId,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        let row = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuilderCatalogError::NotFound)?;
        row.try_into_domain()
    }

    async fn find_builder_image_by_reference(
        &self,
        reference: &BuilderImageReference,
    ) -> Result<BuilderImage, BuilderCatalogError> {
        let row = sqlx::query_as::<_, BuilderImageRow>(
            "SELECT id, key, display_name, image_reference, toolchains,
                    architectures, preparation_state, availability_state,
                    network_ceiling, max_vcpus, max_memory_mib,
                    dependency_policy, provenance, signature_reference,
                    sbom_reference, platform_policy_version
             FROM builder_images WHERE image_reference = $1",
        )
        .bind(reference.as_str())
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(BuilderCatalogError::NotFound)?;
        row.try_into_domain()
    }
}

#[derive(Debug, FromRow)]
struct BuilderImageRow {
    id: Uuid,
    key: String,
    display_name: String,
    image_reference: String,
    toolchains: Value,
    architectures: Vec<String>,
    preparation_state: String,
    availability_state: String,
    network_ceiling: String,
    max_vcpus: i16,
    max_memory_mib: i32,
    dependency_policy: String,
    provenance: Value,
    signature_reference: Option<String>,
    sbom_reference: Option<String>,
    platform_policy_version: String,
}

impl BuilderImageRow {
    fn try_into_domain(self) -> Result<BuilderImage, BuilderCatalogError> {
        let key = BuilderKey::parse(self.key).map_err(invalid_data)?;
        let image_reference =
            BuilderImageReference::parse(self.image_reference).map_err(invalid_data)?;
        let toolchains =
            serde_json::from_value::<Vec<Toolchain>>(self.toolchains).map_err(storage)?;
        let provenance =
            serde_json::from_value::<BuilderProvenance>(self.provenance).map_err(storage)?;
        let preparation = preparation_state(&self.preparation_state).map_err(invalid_data)?;
        let availability = availability_state(&self.availability_state).map_err(invalid_data)?;
        let network_ceiling = network_policy(&self.network_ceiling).map_err(invalid_data)?;
        let dependency_policy = dependency_policy(&self.dependency_policy).map_err(invalid_data)?;
        let max_vcpus = u8::try_from(self.max_vcpus)
            .map_err(|_| invalid_data(BuilderCatalogValueError::InvalidResourceCeiling))?;
        let max_memory_mib = u32::try_from(self.max_memory_mib)
            .map_err(|_| invalid_data(BuilderCatalogValueError::InvalidResourceCeiling))?;
        let entry = BuilderImage {
            id: BuilderImageId::from_uuid(self.id),
            key,
            display_name: self.display_name,
            image_reference,
            toolchains,
            architectures: self.architectures,
            preparation,
            availability,
            network_ceiling,
            max_vcpus,
            max_memory_mib,
            dependency_policy,
            provenance: BuilderProvenance {
                source: provenance.source,
                signature: self.signature_reference.or(provenance.signature),
                sbom: self.sbom_reference.or(provenance.sbom),
            },
            platform_policy_version: self.platform_policy_version,
        };
        entry.validate().map_err(invalid_data)?;
        Ok(entry)
    }
}

fn preparation_state(value: &str) -> Result<PreparationState, BuilderCatalogValueError> {
    match value {
        "ready" => Ok(PreparationState::Ready),
        "preparing" => Ok(PreparationState::Preparing),
        "failed" => Ok(PreparationState::Failed),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn availability_state(value: &str) -> Result<AvailabilityState, BuilderCatalogValueError> {
    match value {
        "available" => Ok(AvailabilityState::Available),
        "unavailable" => Ok(AvailabilityState::Unavailable),
        "retired" => Ok(AvailabilityState::Retired),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn network_policy(value: &str) -> Result<BuildNetworkPolicy, BuilderCatalogValueError> {
    match value {
        "disabled" => Ok(BuildNetworkPolicy::Disabled),
        "broker_only" => Ok(BuildNetworkPolicy::BrokerOnly),
        "egress" => Ok(BuildNetworkPolicy::Egress),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

fn dependency_policy(value: &str) -> Result<DependencyPolicy, BuilderCatalogValueError> {
    match value {
        "vendored_offline" => Ok(DependencyPolicy::VendoredOffline),
        "read_only_platform_cache" => Ok(DependencyPolicy::ReadOnlyPlatformCache),
        "constrained_registry_egress" => Ok(DependencyPolicy::ConstrainedRegistryEgress),
        _ => Err(BuilderCatalogValueError::InvalidStoredValue),
    }
}

const fn invalid_data(error: BuilderCatalogValueError) -> BuilderCatalogError {
    BuilderCatalogError::InvalidData(error)
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> BuilderCatalogError {
    BuilderCatalogError::Storage(Box::new(error))
}
