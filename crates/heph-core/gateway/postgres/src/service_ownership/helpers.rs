//! Shared SQL, lease validation, and row mapping for service ownership.

use gateway_domain::{
    GatewayServiceIdentity, GatewayServiceInstanceLease, GatewayServiceInstanceState,
    GatewayServiceOwner, GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_LEASE,
};
use sqlx::{FromRow, PgPool};
use std::time::Duration;
use time::{Duration as TimeDuration, OffsetDateTime};
use uuid::Uuid;

pub async fn update_state(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    state: &str,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    validate_lease(lease)?;
    owner.validate()?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    lock_gateway(&mut transaction, lease.identity.gateway_id).await?;
    let current = lock_instance(&mut transaction, lease.identity.instance_id).await?;
    let now = database_now(&mut transaction).await?;
    ensure_current_lease(&current, lease, owner, now)?;
    if state == "cleaned" && current.state != "stopping" {
        return Err(GatewayServiceOwnershipError::StaleLease);
    }
    let row = sqlx::query_as::<_, ServiceInstanceRow>(
        "UPDATE gateway_service_instances
            SET state = $2,
                cleaned_at = CASE WHEN $2 = 'cleaned' THEN $3 ELSE cleaned_at END,
                updated_at = now()
          WHERE id = $1
         RETURNING id, gateway_id, revision_id, owner_host_id, owner_uuid,
                   fencing_token, vm_id, state, lease_expires_at, heartbeat_at",
    )
    .bind(current.id)
    .bind(state)
    .bind(now)
    .fetch_one(&mut *transaction)
    .await
    .map_err(|error| storage(&error))?;
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    Ok(row)
}

pub async fn transition_state(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    expected_state: &str,
    next_state: &str,
    reject_active: bool,
    reset_retry: bool,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    validate_lease(lease)?;
    owner.validate()?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
    let gateway = lock_gateway(&mut transaction, lease.identity.gateway_id).await?;
    let current = lock_instance(&mut transaction, lease.identity.instance_id).await?;
    let now = database_now(&mut transaction).await?;
    ensure_current_lease(&current, lease, owner, now)?;
    if current.state != expected_state
        || (reject_active
            && gateway.lifecycle == "enabled"
            && gateway.active_revision_id == Some(lease.identity.revision_id))
    {
        return Err(GatewayServiceOwnershipError::Conflict);
    }
    let row = persist_state(&mut transaction, current.id, next_state, now).await?;
    if reset_retry {
        sqlx::query(
            "UPDATE gateway_service_retry_state
                SET failure_streak = 0, next_retry_at = NULL, updated_at = now()
              WHERE gateway_id = $1 AND revision_id = $2",
        )
        .bind(current.gateway_id)
        .bind(current.revision_id)
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
        let fresh_now = database_now(&mut transaction).await?;
        ensure_current_lease(&current, lease, owner, fresh_now)?;
    }
    transaction
        .commit()
        .await
        .map_err(|error| storage(&error))?;
    Ok(row)
}

pub async fn persist_state(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    instance_id: Uuid,
    state: &str,
    now: OffsetDateTime,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    sqlx::query_as::<_, ServiceInstanceRow>(
        "UPDATE gateway_service_instances
            SET state = $2,
                cleaned_at = CASE WHEN $2 = 'cleaned' THEN $3 ELSE cleaned_at END,
                updated_at = now()
          WHERE id = $1
         RETURNING id, gateway_id, revision_id, owner_host_id, owner_uuid,
                   fencing_token, vm_id, state, lease_expires_at, heartbeat_at",
    )
    .bind(instance_id)
    .bind(state)
    .bind(now)
    .fetch_one(&mut **transaction)
    .await
    .map_err(|error| storage(&error))
}

pub async fn lock_gateway(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    gateway_id: Uuid,
) -> Result<GatewayStateRow, GatewayServiceOwnershipError> {
    sqlx::query_as::<_, GatewayStateRow>(
        "SELECT lifecycle, active_revision_id,
                desired_service_revision_id AS desired_revision_id
           FROM gateways
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(gateway_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| storage(&error))?
    .ok_or(GatewayServiceOwnershipError::StaleLease)
}

pub async fn lock_instance(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    instance_id: Uuid,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    sqlx::query_as::<_, ServiceInstanceRow>(
        "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                fencing_token, vm_id, state, lease_expires_at, heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(instance_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| storage(&error))?
    .ok_or(GatewayServiceOwnershipError::StaleLease)
}

pub async fn lock_exact_instance(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    identity: GatewayServiceIdentity,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    sqlx::query_as::<_, ServiceInstanceRow>(
        "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                fencing_token, vm_id, state, lease_expires_at, heartbeat_at
           FROM gateway_service_instances
          WHERE id = $1 AND gateway_id = $2 AND revision_id = $3
          FOR UPDATE",
    )
    .bind(identity.instance_id)
    .bind(identity.gateway_id)
    .bind(identity.revision_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| storage(&error))?
    .ok_or(GatewayServiceOwnershipError::StaleLease)
}

#[derive(Debug, FromRow)]
pub struct GatewayStateRow {
    pub lifecycle: String,
    pub active_revision_id: Option<Uuid>,
    pub desired_revision_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
pub struct RetryStateRow {
    pub next_retry_at: Option<OffsetDateTime>,
}

pub async fn database_now(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<OffsetDateTime, GatewayServiceOwnershipError> {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| storage(&error))
}

pub fn ensure_current_lease(
    current: &ServiceInstanceRow,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    now: OffsetDateTime,
) -> Result<(), GatewayServiceOwnershipError> {
    if current.id != lease.identity.instance_id
        || current.gateway_id != lease.identity.gateway_id
        || current.revision_id != lease.identity.revision_id
        || current.owner_host_id != lease.owner_host_id
        || current.owner_uuid != lease.owner_uuid
        || current.owner_host_id != owner.host_id
        || current.owner_uuid != owner.owner_uuid
        || current.fencing_token != lease.fencing_token
        || current.vm_id != lease.vm_id
        || current.state == "cleaned"
        || current.lease_expires_at <= now
    {
        return Err(GatewayServiceOwnershipError::StaleLease);
    }
    Ok(())
}

pub const fn validate_identity(
    gateway_id: Uuid,
    revision_id: Uuid,
) -> Result<(), GatewayServiceOwnershipError> {
    if gateway_id.is_nil() || revision_id.is_nil() {
        Err(GatewayServiceOwnershipError::InvalidArgument)
    } else {
        Ok(())
    }
}

pub fn validate_lease(
    lease: &GatewayServiceInstanceLease,
) -> Result<(), GatewayServiceOwnershipError> {
    validate_identity(lease.identity.gateway_id, lease.identity.revision_id)?;
    if lease.identity.instance_id.is_nil()
        || lease.owner_uuid.is_nil()
        || lease.fencing_token <= 0
        || lease.vm_id != service_vm_id(lease.identity.instance_id)
    {
        return Err(GatewayServiceOwnershipError::InvalidArgument);
    }
    GatewayServiceOwner::new(lease.owner_host_id.clone(), lease.owner_uuid)?;
    Ok(())
}

#[derive(Debug, FromRow)]
pub struct ServiceInstanceRow {
    pub id: Uuid,
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
    pub owner_host_id: String,
    pub owner_uuid: Uuid,
    pub fencing_token: i64,
    pub vm_id: String,
    pub state: String,
    pub lease_expires_at: OffsetDateTime,
    pub heartbeat_at: OffsetDateTime,
}

impl ServiceInstanceRow {
    pub fn into_lease(self) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        Ok(GatewayServiceInstanceLease {
            identity: GatewayServiceIdentity {
                instance_id: self.id,
                gateway_id: self.gateway_id,
                revision_id: self.revision_id,
            },
            owner_host_id: self.owner_host_id,
            owner_uuid: self.owner_uuid,
            fencing_token: self.fencing_token,
            state: parse_state(&self.state)?,
            vm_id: self.vm_id,
            lease_expires_at: self.lease_expires_at,
            heartbeat_at: self.heartbeat_at,
        })
    }
}

pub fn parse_state(
    value: &str,
) -> Result<GatewayServiceInstanceState, GatewayServiceOwnershipError> {
    match value {
        "provisioning" => Ok(GatewayServiceInstanceState::Provisioning),
        "starting" => Ok(GatewayServiceInstanceState::Starting),
        "ready" => Ok(GatewayServiceInstanceState::Ready),
        "draining" => Ok(GatewayServiceInstanceState::Draining),
        "stopping" => Ok(GatewayServiceInstanceState::Stopping),
        "failed" => Ok(GatewayServiceInstanceState::Failed),
        "cleaned" => Ok(GatewayServiceInstanceState::Cleaned),
        _ => Err(GatewayServiceOwnershipError::Unavailable),
    }
}

pub fn service_vm_id(instance_id: Uuid) -> String {
    format!("gateway-service-{instance_id}")
}

pub fn checked_lease_duration(
    value: Duration,
) -> Result<TimeDuration, GatewayServiceOwnershipError> {
    if value.is_zero() || value > MAX_SERVICE_OWNERSHIP_LEASE {
        return Err(GatewayServiceOwnershipError::InvalidArgument);
    }
    TimeDuration::try_from(value).map_err(|_| GatewayServiceOwnershipError::InvalidArgument)
}

pub fn storage(error: &sqlx::Error) -> GatewayServiceOwnershipError {
    if let sqlx::Error::Database(database) = error {
        if matches!(database.code().as_deref(), Some("23505" | "23000")) {
            return GatewayServiceOwnershipError::Conflict;
        }
    }
    GatewayServiceOwnershipError::Unavailable
}
