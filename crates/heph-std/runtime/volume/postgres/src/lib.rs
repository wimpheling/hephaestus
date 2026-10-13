//! `PostgreSQL` metadata, lease, and optimistic-fencing adapter for volumes.

pub use ready_retention::PostgresReadyRetentionRepository;

use sqlx::PgPool;

mod errors;
mod models;
mod owned_provisioning;
mod owned_registration;
mod owned_root_history;
mod partial_retention;
mod provisioning;
mod ready_retention;
mod repository;
mod resources;
mod run_volumes;
mod scalar_schema;

pub use scalar_schema::ScalarMetadataSchema;

/// PostgreSQL-backed volume metadata repository.
#[derive(Clone)]
pub struct PostgresVolumeMetadataRepository {
    pool: PgPool,
    scalar_schema: ScalarMetadataSchema,
    invocation_configuration: Option<release_service::InstanceExecutionConfiguration>,
}

impl PostgresVolumeMetadataRepository {
    /// Creates a repository over a `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self {
            pool,
            scalar_schema: ScalarMetadataSchema::Public107,
            invocation_configuration: None,
        }
    }

    /// Selects the trusted scalar query shape for a reviewed schema.
    ///
    /// The default uses public107 queries. `Profiled` requires the private
    /// profile/scalar compatibility functions; it still checks each persisted
    /// Run profile and conveys no caller or provider cleanup authority.
    #[must_use]
    pub const fn with_scalar_schema(mut self, schema: ScalarMetadataSchema) -> Self {
        self.scalar_schema = schema;
        self
    }

    /// Selects qualified Invocation lease metadata for the private119 schema.
    ///
    /// The default is unsupported. The checked configuration must come from
    /// trusted startup validation of the actual owners. This data comparison
    /// grants no caller rights or physical provider/backing proof; canonical
    /// cleanup still requires its actual held-provider observations.
    #[must_use]
    pub fn with_qualified_invocation_configuration(
        mut self,
        configuration: release_service::InstanceExecutionConfiguration,
    ) -> Self {
        self.invocation_configuration = Some(configuration);
        self
    }

    /// Applies schema migrations owned by the application composition root.
    ///
    /// # Errors
    ///
    /// Returns an error when migrations cannot be applied.
    pub async fn initialize(&self) -> Result<(), volume_trait::VolumeError> {
        sqlx::migrate!("../../../../../migrations")
            .run(&self.pool)
            .await
            .map_err(errors::metadata)
    }
}
