//! Claim and renewal operations for gateway service ownership.

use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceOwner, GatewayServiceOwnershipError,
};
use sqlx::PgPool;
use std::time::Duration;
use uuid::Uuid;

use super::helpers::{
    RetryStateRow, ServiceInstanceRow, checked_lease_duration, database_now, ensure_current_lease,
    lock_gateway, lock_instance, service_vm_id, storage, validate_identity, validate_lease,
};

pub async fn claim_new(
    pool: &PgPool,
    gateway_id: Uuid,
    revision_id: Uuid,
    owner: &GatewayServiceOwner,
    lease_duration: Duration,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    validate_identity(gateway_id, revision_id)?;
    owner.validate()?;
    let lease_duration = checked_lease_duration(lease_duration)?;
    let instance_id = Uuid::new_v4();
    let vm_id = service_vm_id(instance_id);
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    let eligible: Option<bool> = sqlx::query_scalar(
        "SELECT TRUE
           FROM gateways AS gateway
           JOIN gateway_revisions AS revision
             ON revision.id = $2
            AND revision.gateway_id = gateway.id
          WHERE gateway.id = $1
            AND gateway.lifecycle = 'enabled'
            AND revision.handler_contract = 'http.service.v1'
            AND (gateway.desired_service_revision_id = $2
                 OR gateway.active_revision_id = $2)
          FOR UPDATE OF gateway",
    )
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    if eligible.is_none() {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    let retry_at = sqlx::query_as::<_, RetryStateRow>(
        "SELECT next_retry_at
           FROM gateway_service_retry_state
          WHERE gateway_id = $1 AND revision_id = $2
          FOR UPDATE",
    )
    .bind(gateway_id)
    .bind(revision_id)
    .fetch_optional(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    let now = database_now(&mut transaction).await?;
    if retry_at
        .and_then(|row| row.next_retry_at)
        .is_some_and(|retry_at| retry_at > now)
    {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    let expires_at = now
        .checked_add(lease_duration)
        .ok_or(GatewayServiceOwnershipError::InvalidArgument)?;
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid,
             fencing_token, vm_id, state, lease_expires_at, heartbeat_at)
         VALUES ($1, $2, $3, $4, $5, 1, $6, 'provisioning', $7, $8)
         RETURNING id, gateway_id, revision_id, owner_host_id, owner_uuid,
                   fencing_token, vm_id, state, lease_expires_at,
                   heartbeat_at",
    )
    .bind(instance_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(&owner.host_id)
    .bind(owner.owner_uuid)
    .bind(&vm_id)
    .bind(expires_at)
    .bind(now)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    row.into_lease()
}

pub async fn renew(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    lease_duration: Duration,
) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
    validate_lease(lease)?;
    owner.validate()?;
    let lease_duration = checked_lease_duration(lease_duration)?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    lock_gateway(&mut transaction, lease.identity.gateway_id).await?;
    let current = lock_instance(&mut transaction, lease.identity.instance_id).await?;
    let now = database_now(&mut transaction).await?;
    ensure_current_lease(&current, lease, owner, now)?;
    let expires_at = now
        .checked_add(lease_duration)
        .ok_or(GatewayServiceOwnershipError::InvalidArgument)?;
    let expires_at = expires_at.max(current.lease_expires_at);
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "UPDATE gateway_service_instances
            SET heartbeat_at = $2, lease_expires_at = $3, updated_at = now()
          WHERE id = $1
         RETURNING id, gateway_id, revision_id, owner_host_id, owner_uuid,
                   fencing_token, vm_id, state, lease_expires_at,
                   heartbeat_at",
    )
    .bind(current.id)
    .bind(now)
    .bind(expires_at)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    row.into_lease()
}
