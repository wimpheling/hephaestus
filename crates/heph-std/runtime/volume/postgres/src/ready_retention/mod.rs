//! Separate closed Ready worker adapter; configured labels are comparison data.
mod context;
mod facts;
mod retained_reuse;
use async_trait::async_trait;
use sqlx::PgPool;
use uuid::Uuid;
use volume_trait::{
    OwnedBackingObservation, OwnedProvisioningClaim, OwnedReadyRetentionContext,
    OwnedReadyRetentionFact, VolumeError, VolumeReadyRetentionRepository,
};

/// Worker adapter with explicit trusted actual VM scope comparison pins.
///
/// Populate these from the configured provider's checked scope, never client
/// input, historical guesses or the disk namespace. This object issues no VM
/// authority and conveys no filesystem custody or shutdown proof.
#[derive(Clone)]
pub struct PostgresReadyRetentionRepository {
    pool: PgPool,
    namespace: String,
    host: String,
}
impl PostgresReadyRetentionRepository {
    /// Selects the reviewed Ready successor with trusted exact VM comparison pins.
    ///
    /// # Errors
    /// Unsafe or unbounded opaque namespace/host labels deny composition.
    pub fn new(pool: PgPool, namespace: String, host: String) -> Result<Self, VolumeError> {
        if namespace.is_empty()
            || namespace.len() > 128
            || matches!(namespace.as_str(), "." | "..")
            || !namespace.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
            || host.is_empty()
            || host.len() > 128
            || matches!(host.as_str(), "." | "..")
            || !host.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')
            })
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(Self {
            pool,
            namespace,
            host,
        })
    }
}
#[async_trait]
impl VolumeReadyRetentionRepository for PostgresReadyRetentionRepository {
    async fn ready_retention_context(
        &self,
        claim: &OwnedProvisioningClaim,
        closure: Uuid,
    ) -> Result<OwnedReadyRetentionContext, VolumeError> {
        let mut tx = self.pool.begin().await.map_err(crate::errors::metadata)?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
            .execute(&mut *tx)
            .await
            .map_err(crate::errors::metadata)?;
        let value = context::load(self, &mut tx, claim, closure).await?;
        tx.commit().await.map_err(crate::errors::metadata)?;
        Ok(value)
    }
    async fn record_ready_retention_observation(
        &self,
        context: &OwnedReadyRetentionContext,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedReadyRetentionFact, VolumeError> {
        facts::record(self, context, observation).await
    }
}
