//! `PostgreSQL` durable ownership for long-lived gateway service instances.

use async_trait::async_trait;
use gateway_edge::{
    GatewayServiceClaimResolutionStore, GatewayServiceIdentity, GatewayServiceInstanceLease,
    GatewayServiceInstanceState, GatewayServiceOwner, GatewayServiceOwnership,
    GatewayServiceOwnershipError, MAX_SERVICE_OWNERSHIP_BATCH, MAX_SERVICE_OWNERSHIP_LEASE,
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

    async fn mark_starting(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transition_state(
            &self.pool,
            lease,
            owner,
            "provisioning",
            "starting",
            false,
            false,
        )
        .await?
        .into_lease()
    }

    async fn mark_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transition_state(&self.pool, lease, owner, "starting", "ready", false, true)
            .await?
            .into_lease()
    }

    async fn mark_draining(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<GatewayServiceInstanceLease, GatewayServiceOwnershipError> {
        transition_state(&self.pool, lease, owner, "ready", "draining", true, false)
            .await?
            .into_lease()
    }

    async fn promote_ready(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<Option<Uuid>, GatewayServiceOwnershipError> {
        validate_lease(lease)?;
        owner.validate()?;
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
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

    async fn mark_cleaned(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
    ) -> Result<(), GatewayServiceOwnershipError> {
        update_state(&self.pool, lease, owner, "cleaned").await?;
        Ok(())
    }
}

#[async_trait]
impl GatewayServiceClaimResolutionStore for PostgresGatewayServiceOwnership {
    async fn resolve_revision_claim(
        &self,
        gateway_id: Uuid,
        revision_id: Uuid,
    ) -> Result<Option<GatewayServiceInstanceLease>, GatewayServiceOwnershipError> {
        validate_identity(gateway_id, revision_id)?;
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
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

async fn transition_state(
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

async fn persist_state(
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

async fn lock_gateway(
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

#[derive(Debug, FromRow)]
struct GatewayStateRow {
    lifecycle: String,
    active_revision_id: Option<Uuid>,
    desired_revision_id: Option<Uuid>,
}

#[derive(Debug, FromRow)]
struct RetryStateRow {
    next_retry_at: Option<OffsetDateTime>,
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
pub struct ServiceInstanceRow {
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
