//! `PostgreSQL` metadata, lease, and optimistic-fencing adapter for volumes.

use sqlx::PgPool;

mod errors;
mod models;
mod repository;

/// PostgreSQL-backed volume metadata repository.
#[derive(Clone)]
pub struct PostgresVolumeMetadataRepository {
    pool: PgPool,
}

impl PostgresVolumeMetadataRepository {
    /// Creates a repository over a `PostgreSQL` pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
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
