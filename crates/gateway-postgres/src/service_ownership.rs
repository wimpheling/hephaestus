//! `PostgreSQL` durable ownership for long-lived gateway service instances.

use async_trait::async_trait;
use gateway_edge::{
    GatewayServiceIdentity, GatewayServiceInstanceLease, GatewayServiceInstanceState,
    GatewayServiceOwner, GatewayServiceOwnership, GatewayServiceOwnershipError,
    MAX_SERVICE_OWNERSHIP_BATCH, MAX_SERVICE_OWNERSHIP_LEASE,
};
use sqlx::{FromRow, PgPool};
use std::time::Duration;
use time::{Duration as TimeDuration, OffsetDateTime};
use uuid::Uuid;

/// `PostgreSQL` adapter for gateway service ownership and fencing.
#[derive(Clone)]
pub struct PostgresGatewayServiceOwnership {
    pool: PgPool,
}

impl PostgresGatewayServiceOwnership {
    /// Creates an ownership adapter over the worker-authorized pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayServiceOwnership for PostgresGatewayServiceOwnership {
    async fn claim_new(
        &self,
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
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
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
        let now = database_now(&mut transaction).await?;
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

    async fn renew(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        validate_lease(lease)?;
        owner.validate()?;
        let lease_duration = checked_lease_duration(lease_duration)?;
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
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

    async fn claim_expired(
        &self,
        owner: &GatewayServiceOwner,
        lease_duration: Duration,
        limit: usize,
    ) -> Result<Vec<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        owner.validate()?;
        let lease_duration = checked_lease_duration(lease_duration)?;
        if !(1..=MAX_SERVICE_OWNERSHIP_BATCH).contains(&limit) {
            return Err(GatewayServiceOwnershipError::InvalidArgument);
        }
        let limit =
            i64::try_from(limit).map_err(|_| GatewayServiceOwnershipError::InvalidArgument)?;
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
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

    async fn mark_stopping(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        update_state(&self.pool, lease, owner, "stopping")
            .await?
            .into_lease()
    }

    async fn mark_cleaned(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        update_state(&self.pool, lease, owner, "cleaned").await?;
        Ok(())
    }
}

async fn update_state(
    pool: &PgPool,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    state: &str,
) -> Result<ServiceInstanceRow, GatewayServiceOwnershipError> {
    validate_lease(lease)?;
    owner.validate()?;
    let mut transaction = pool.begin().await.map_err(|error| storage(&error))?;
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

async fn lock_instance(
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

async fn database_now(
    transaction: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<OffsetDateTime, GatewayServiceOwnershipError> {
    sqlx::query_scalar("SELECT clock_timestamp()")
        .fetch_one(&mut **transaction)
        .await
        .map_err(|error| storage(&error))
}

fn ensure_current_lease(
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

const fn validate_identity(
    gateway_id: Uuid,
    revision_id: Uuid,
) -> Result<(), GatewayServiceOwnershipError> {
    if gateway_id.is_nil() || revision_id.is_nil() {
        Err(GatewayServiceOwnershipError::InvalidArgument)
    } else {
        Ok(())
    }
}

fn validate_lease(lease: &GatewayServiceInstanceLease) -> Result<(), GatewayServiceOwnershipError> {
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
struct ServiceInstanceRow {
    id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    owner_host_id: String,
    owner_uuid: Uuid,
    fencing_token: i64,
    vm_id: String,
    state: String,
    lease_expires_at: OffsetDateTime,
    heartbeat_at: OffsetDateTime,
}

impl ServiceInstanceRow {
    fn into_lease(self) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
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

fn parse_state(value: &str) -> Result<GatewayServiceInstanceState, GatewayServiceOwnershipError> {
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

fn service_vm_id(instance_id: Uuid) -> String {
    format!("gateway-service-{instance_id}")
}

fn checked_lease_duration(value: Duration) -> Result<TimeDuration, GatewayServiceOwnershipError> {
    if value.is_zero() || value > MAX_SERVICE_OWNERSHIP_LEASE {
        return Err(GatewayServiceOwnershipError::InvalidArgument);
    }
    TimeDuration::try_from(value).map_err(|_| GatewayServiceOwnershipError::InvalidArgument)
}

fn storage(error: &sqlx::Error) -> GatewayServiceOwnershipError {
    if let sqlx::Error::Database(database) = error {
        if matches!(database.code().as_deref(), Some("23505" | "23000")) {
            return GatewayServiceOwnershipError::Conflict;
        }
    }
    GatewayServiceOwnershipError::Unavailable
}
