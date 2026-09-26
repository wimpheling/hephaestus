//! Revision claim resolution.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceClaimResolutionStore, GatewayServiceInstanceLease, GatewayServiceOwnershipError,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::helpers::{ServiceInstanceRow, storage, validate_identity};

pub async fn resolve_revision_claim(
    pool: &PgPool,
    gateway_id: Uuid,
    revision_id: Uuid,
) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
    validate_identity(gateway_id, revision_id)?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    // Keep this explicit: a configurable pool must not silently weaken the
    // claim-resolution barrier by using a stronger or session-only mode.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
    let gateway: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
            .bind(gateway_id)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(|error| storage(&error))?;
    if gateway.is_none() {
        transaction
            .commit()
            .await
            .map_err(|error| storage(&error))?;
        return Ok(None);
    }
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                fencing_token, vm_id, state, lease_expires_at, heartbeat_at
           FROM gateway_service_instances
          WHERE gateway_id = $1
            AND revision_id = $2
            AND state <> 'cleaned'
          ORDER BY fencing_token DESC, id DESC
          LIMIT 1",
    )
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    row.map(ServiceInstanceRow::into_lease).transpose()
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for super::PostgresGatewayServiceOwnership {
    async fn resolve_revision_claim(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        resolve_revision_claim(&self.pool, gateway_id, revision_id).await
    }
}
