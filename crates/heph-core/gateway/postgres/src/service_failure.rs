//! `PostgreSQL` adapter for redacted service failure recording and backoff.

use async_trait::async_trait;
use gateway_edge::{
    GatewayServiceFailure, GatewayServiceFailureStore, GatewayServiceFailureStoreError,
    GatewayServiceInstanceLease, GatewayServiceOwner,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use time::{Duration, OffsetDateTime};
use uuid::Uuid;

/// `PostgreSQL` durable failure store for long-lived gateway services.
#[derive(Clone)]
pub struct PostgresGatewayServiceFailureStore {
    pool: PgPool,
}

impl PostgresGatewayServiceFailureStore {
    /// Creates a failure store over the worker-authorized pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayServiceFailureStore for PostgresGatewayServiceFailureStore {
    async fn record_failure(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        failure: GatewayServiceFailure,
    ) -> Result<(), GatewayServiceFailureStoreError> {
        validate_lease(lease)?;
        owner
            .validate()
            .map_err(|_| GatewayServiceFailureStoreError::InvalidArgument)?;
        failure.validate()?;
        let mut transaction = self.pool.begin().await.map_err(|error| storage(&error))?;
        lock_gateway(&mut transaction, lease.identity.gateway_id).await?;
        let current = lock_instance(&mut transaction, lease.identity.instance_id).await?;
        let previous = sqlx::query_as::<_, RetryStateRow>(
            "SELECT failure_streak
               FROM gateway_service_retry_state
              WHERE gateway_id = $1 AND revision_id = $2
              FOR UPDATE",
        )
        .bind(current.gateway_id)
        .bind(current.revision_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
        // The retry row may have waited behind another reporter. Re-read the
        // database clock after that wait before accepting this lease or
        // treating a duplicate report as harmless.
        let now = database_now(&mut transaction).await?;
        ensure_current_lease(&current, lease, owner, now)?;

        // A failure is terminal for this launch attempt. A repeated report
        // (including a different provider category) is deliberately a no-op.
        if current.failure_code.is_some() {
            transaction
                .commit()
                .await
                .map_err(|error| storage(&error))?;
            return Ok(());
        }
        let failure_streak = previous
            .as_ref()
            .map_or(1, |row| row.failure_streak.saturating_add(1).min(32));
        let retry_at = now
            .checked_add(retry_delay(failure_streak))
            .ok_or(GatewayServiceFailureStoreError::InvalidArgument)?;
        sqlx::query(
            "INSERT INTO gateway_service_retry_state
                (gateway_id, revision_id, failure_streak, next_retry_at, updated_at)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (gateway_id, revision_id) DO UPDATE
                 SET failure_streak = EXCLUDED.failure_streak,
                     next_retry_at = EXCLUDED.next_retry_at,
                     updated_at = now()",
        )
        .bind(current.gateway_id)
        .bind(current.revision_id)
        .bind(failure_streak)
        .bind(retry_at)
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
        sqlx::query(
            "UPDATE gateway_service_instances
                SET failure_code = $2, failed_at = $3,
                    exit_code = $4, exit_signal = $5, updated_at = now()
              WHERE id = $1 AND failure_code IS NULL",
        )
        .bind(current.id)
        .bind(failure.code.as_str())
        .bind(now)
        .bind(failure.exit_code)
        .bind(failure.exit_signal)
        .execute(&mut *transaction)
        .await
        .map_err(|error| storage(&error))?;
        transaction.commit().await.map_err(|error| storage(&error))
    }
}

const fn retry_delay(failure_streak: i32) -> Duration {
    let seconds = match failure_streak {
        0 | 1 => 1,
        2 => 2,
        3 => 4,
        4 => 8,
        5 => 16,
        6 => 32,
        _ => 60,
    };
    Duration::seconds(seconds)
}

async fn lock_gateway(
    transaction: &mut Transaction<'_, Postgres>,
    gateway_id: Uuid,
) -> Result<(), GatewayServiceFailureStoreError> {
    let exists: Option<Uuid> =
        sqlx::query_scalar("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
            .bind(gateway_id)
            .fetch_optional(&mut **transaction)
            .await
            .map_err(|error| storage(&error))?;
    exists
        .map(|_| ())
        .ok_or(GatewayServiceFailureStoreError::StaleLease)
}

async fn lock_instance(
    transaction: &mut Transaction<'_, Postgres>,
    instance_id: Uuid,
) -> Result<ServiceInstanceRow, GatewayServiceFailureStoreError> {
    sqlx::query_as::<_, ServiceInstanceRow>(
        "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                fencing_token, vm_id, state, lease_expires_at, heartbeat_at,
                failure_code
           FROM gateway_service_instances
          WHERE id = $1
          FOR UPDATE",
    )
    .bind(instance_id)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(|error| storage(&error))?
    .ok_or(GatewayServiceFailureStoreError::StaleLease)
}

async fn database_now(
    transaction: &mut Transaction<'_, Postgres>,
) -> Result<OffsetDateTime, GatewayServiceFailureStoreError> {
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
) -> Result<(), GatewayServiceFailureStoreError> {
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
        return Err(GatewayServiceFailureStoreError::StaleLease);
    }
    Ok(())
}

fn validate_lease(
    lease: &GatewayServiceInstanceLease,
) -> Result<(), GatewayServiceFailureStoreError> {
    if lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
        || lease.identity.instance_id.is_nil()
        || lease.owner_uuid.is_nil()
        || lease.fencing_token <= 0
        || lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id)
        || lease.owner_host_id.is_empty()
        || lease.owner_host_id.len() > 200
        || lease.owner_host_id != lease.owner_host_id.trim()
        || lease
            .owner_host_id
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(GatewayServiceFailureStoreError::InvalidArgument);
    }
    Ok(())
}

fn storage(error: &sqlx::Error) -> GatewayServiceFailureStoreError {
    tracing::error!(error = %error, "gateway service failure storage operation failed");
    GatewayServiceFailureStoreError::Unavailable
}

#[derive(Debug, FromRow)]
struct RetryStateRow {
    failure_streak: i32,
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
    failure_code: Option<String>,
}
