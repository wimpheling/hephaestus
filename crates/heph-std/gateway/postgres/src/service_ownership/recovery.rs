//! Expired ownership recovery and exact instance resolution.

use super::helpers::{
    ServiceInstanceRow, checked_lease_duration, database_now, lock_exact_instance, lock_gateway,
    lock_instance, storage, validate_identity, validate_lease,
};
use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceExpiredClaimRecovery, GatewayServiceIdentity, GatewayServiceInstanceLease,
    GatewayServiceOwner, GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_BATCH,
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

pub async fn claim_expired(
    pool: &PgPool,
    owner: &GatewayServiceOwner,
    lease_duration: Duration,
    limit: usize,
) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
    owner.validate()?;
    let lease_duration = checked_lease_duration(lease_duration)?;
    if !(1..=MAX_SERVICE_OWNERSHIP_BATCH).contains(&limit) {
        return Err(GatewayServiceOwnershipError::InvalidArgument);
    }
    let limit = i64::try_from(limit).map_err(|_| GatewayServiceOwnershipError::InvalidArgument)?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    let candidate_now = database_now(&mut transaction).await?;
    let candidates: Vec<(Uuid, Uuid)> = sqlx::query_as(
        "SELECT instance.gateway_id, instance.id
           FROM gateway_service_instances AS instance
          WHERE instance.owner_host_id = $1
            AND instance.state <> 'cleaned'
            AND instance.lease_expires_at <= $2
          ORDER BY instance.gateway_id, instance.id
          LIMIT $3",
    )
    .bind(&owner.host_id)
    .bind(candidate_now)
    .bind(limit)
    .fetch_all(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    let mut recovered = Vec::with_capacity(candidates.len());
    for (gateway_id, instance_id) in candidates {
        // Always lock the gateway before the instance. This is the same
        // aggregate lock order used by claim_new.
        let gateway_locked: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM gateways WHERE id = $1 FOR UPDATE SKIP LOCKED")
                .bind(gateway_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(|error| storage(&error))?;
        if gateway_locked.is_none() {
            continue;
        }
        let current = lock_instance(&mut transaction, instance_id).await?;
        let now = database_now(&mut transaction).await?;
        if current.gateway_id != gateway_id
            || current.owner_host_id != owner.host_id
            || current.state == "cleaned"
            || current.lease_expires_at > now
        {
            continue;
        }
        let fencing_token = current
            .fencing_token
            .checked_add(1)
            .ok_or(GatewayServiceOwnershipError::Unavailable)?;
        let expires_at = now
            .checked_add(lease_duration)
            .ok_or(GatewayServiceOwnershipError::InvalidArgument)?;
        let row = sqlx::query_as::<_, ServiceInstanceRow>(
            "UPDATE gateway_service_instances
                SET owner_uuid = $2, fencing_token = $3,
                    state = 'stopping', heartbeat_at = $4,
                    lease_expires_at = $5, updated_at = now()
              WHERE id = $1
             RETURNING id, gateway_id, revision_id, owner_host_id,
                       owner_uuid, fencing_token, vm_id, state,
                       lease_expires_at, heartbeat_at",
        )
        .bind(current.id)
        .bind(owner.owner_uuid)
        .bind(fencing_token)
        .bind(now)
        .bind(expires_at)
        .fetch_one(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
        recovered.push(row.into_lease()?);
    }
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    Ok(recovered)
}

pub async fn resolve_exact_instance(
    pool: &PgPool,
    identity: GatewayServiceIdentity,
) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
    validate_identity(identity.gateway_id, identity.revision_id)?;
    if identity.instance_id.is_nil() {
        return Err(GatewayServiceOwnershipError::InvalidArgument);
    }
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    // Serialize this read with claim and takeover mutations. In particular,
    // an empty result is authoritative only after the gateway lock is held.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL READ COMMITTED")
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
    lock_gateway(&mut transaction, identity.gateway_id).await?;
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                fencing_token, vm_id, state, lease_expires_at, heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3",
    )
    .bind(identity.instance_id)
    .bind(identity.gateway_id)
    .bind(identity.revision_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    let lease = row.map(ServiceInstanceRow::into_lease).transpose()?;
    if let Some(lease) = &lease {
        validate_lease(lease)?;
    }
    Ok(lease)
}

pub async fn claim_expired_instance(
    pool: &PgPool,
    previous: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    lease_duration: Duration,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    validate_lease(previous)?;
    owner.validate()?;
    let lease_duration = checked_lease_duration(lease_duration)?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    // Recovery follows the aggregate lock order used by every ownership
    // mutation: gateway first, then the exact instance row.
    lock_gateway(&mut transaction, previous.identity.gateway_id).await?;
    let current = lock_exact_instance(&mut transaction, previous.identity).await?;
    let now = database_now(&mut transaction).await?;
    if current.id != previous.identity.instance_id
        || current.gateway_id != previous.identity.gateway_id
        || current.revision_id != previous.identity.revision_id
        || current.owner_host_id != previous.owner_host_id
        || current.owner_host_id != owner.host_id
        || current.owner_uuid != previous.owner_uuid
        || current.fencing_token != previous.fencing_token
        || current.vm_id != previous.vm_id
        || current.state == "cleaned"
        || current.lease_expires_at > now
    {
        return Err(GatewayServiceOwnershipError::StaleLease);
    }
    let fencing_token = current
        .fencing_token
        .checked_add(1)
        .ok_or(GatewayServiceOwnershipError::Unavailable)?;
    let expires_at = now
        .checked_add(lease_duration)
        .ok_or(GatewayServiceOwnershipError::InvalidArgument)?;
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "UPDATE gateway_service_instances
            SET owner_uuid = $2, fencing_token = $3,
                state = 'stopping', heartbeat_at = $4,
                lease_expires_at = $5, updated_at = now()
          WHERE id = $1
         RETURNING id, gateway_id, revision_id, owner_host_id,
                   owner_uuid, fencing_token, vm_id, state,
                   lease_expires_at, heartbeat_at",
    )
    .bind(current.id)
    .bind(owner.owner_uuid)
    .bind(fencing_token)
    .bind(now)
    .bind(expires_at)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    let lease = row.into_lease()?;
    validate_lease(&lease)?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    Ok(lease)
}

#[async_trait]
impl GatewayServiceExpiredClaimRecovery for super::PostgresGatewayServiceOwnership {
    async fn resolve_exact_instance(
        &self,
        identity: GatewayServiceIdentity,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        resolve_exact_instance(&self.pool, identity).await
    }

    async fn claim_expired_instance(
        &self,
        previous: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        claim_expired_instance(&self.pool, previous, owner, lease_duration).await
    }
}
