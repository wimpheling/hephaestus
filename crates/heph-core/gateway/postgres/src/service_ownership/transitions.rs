//! State transitions for gateway service ownership.

use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnershipError,
};
use sqlx::PgPool;
use uuid::Uuid;

use super::helpers::{
    database_now, ensure_current_lease, lock_gateway, lock_instance, storage, transition_state,
    update_state, validate_lease,
};

pub async fn mark_stopping(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    update_state(pool, lease, owner, "stopping")
        .await?
        .into_lease()
}

pub async fn mark_starting(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    transition_state(pool, lease, owner, "provisioning", "starting", false, false)
        .await?
        .into_lease()
}

pub async fn mark_ready(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    transition_state(pool, lease, owner, "starting", "ready", false, true)
        .await?
        .into_lease()
}

pub async fn mark_draining(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    transition_state(pool, lease, owner, "ready", "draining", true, false)
        .await?
        .into_lease()
}

pub async fn promote_ready(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
    validate_lease(lease)?;
    owner.validate()?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    let gateway = lock_gateway(&mut transaction, lease.identity.gateway_id).await?;
    let current = lock_instance(&mut transaction, lease.identity.instance_id).await?;
    if current.state != "ready" {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    if gateway.lifecycle != "enabled"
        || gateway.desired_revision_id != Some(lease.identity.revision_id)
    {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    let publication = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
               FROM gateway_revisions AS revision
               JOIN releases AS release
                 ON release.id = revision.release_id
                AND release.repository_id = revision.repository_id
              WHERE revision.id = $1
                AND revision.gateway_id = $2
                AND revision.handler_contract = 'http.service.v1'
                AND release.state = 'published'
              FOR UPDATE OF release
         )",
    )
    .bind(lease.identity.revision_id)
    .bind(lease.identity.gateway_id)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    if !publication {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    // The release row may have been locked behind a concurrent revoke.
    // Read the clock only after that wait, while the instance remains
    // locked, so an expired owner cannot promote a stale candidate.
    let now = database_now(&mut transaction).await?;
    ensure_current_lease(&current, lease, owner, now)?;
    let previous = gateway.active_revision_id;
    if previous != Some(lease.identity.revision_id) {
        sqlx::query(
            "UPDATE gateways
                SET active_revision_id = $2, updated_at = now()
              WHERE id = $1",
        )
        .bind(lease.identity.gateway_id)
        .bind(lease.identity.revision_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    Ok(previous)
}

pub async fn mark_cleaned(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
) -> Result<(), GatewayServiceOwnershipError> {
    update_state(pool, lease, owner, "cleaned").await?;
    Ok(())
}
