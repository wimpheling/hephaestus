//! Worker-only durable append adapter for opt-in application service logs.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome,
    GatewayServiceLogStore, GatewayServiceLogStoreError, GatewayServiceOwner,
    MAX_SERVICE_LOG_INSTANCE_BYTES, MAX_SERVICE_LOG_INSTANCE_CHUNKS, MAX_SERVICE_LOG_PROJECT_BYTES,
    MAX_SERVICE_LOG_PROJECT_CHUNKS, MAX_SERVICE_OWNER_HOST_BYTES,
};
use sqlx::{FromRow, PgPool};
use time::OffsetDateTime;
use uuid::Uuid;
use vm_trait::LogStream;

// These platform caps are bounded well below `i64::MAX`; the narrow casts
// keep the SQL arithmetic in the same type as PostgreSQL bigint.
#[allow(clippy::cast_possible_wrap)]
const MAX_INSTANCE_BYTES: i64 = MAX_SERVICE_LOG_INSTANCE_BYTES as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_INSTANCE_CHUNKS: i64 = MAX_SERVICE_LOG_INSTANCE_CHUNKS as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_PROJECT_BYTES: i64 = MAX_SERVICE_LOG_PROJECT_BYTES as i64;
#[allow(clippy::cast_possible_wrap)]
const MAX_PROJECT_CHUNKS: i64 = MAX_SERVICE_LOG_PROJECT_CHUNKS as i64;
const MAX_PROJECT_EPOCHS: i32 = 128;
const RETENTION_HOURS: i64 = 24;
const PRESSURE_HIGH_NUMERATOR: i64 = 7;
const PRESSURE_HIGH_DENOMINATOR: i64 = 8;
const PRESSURE_LOW_NUMERATOR: i64 = 3;
const PRESSURE_LOW_DENOMINATOR: i64 = 4;

mod maintenance_deletion;
mod maintenance_project;
mod maintenance_projects;
mod maintenance_queries;

pub use maintenance_deletion::{
    delete_gc_log_epochs, delete_log_chunks, has_more_maintenance_work, load_gc_log_epochs,
    maintenance_storage, maintenance_usize, pressure_above_low_instance,
    pressure_above_low_project, pressure_needed_instance, pressure_needed_project,
};
pub use maintenance_queries::{
    DeletedChunks, MaintenanceChunk, MaintenanceEpoch, MaintenanceProjectUsage, MaintenanceUsage,
    load_instance_epochs, load_instance_usage, load_log_chunks, lock_retention_scopes,
    same_chunk_identity,
};

/// `PostgreSQL` adapter for the worker-owned application log append port.
#[derive(Clone)]
pub struct PostgresGatewayServiceLogStore {
    pool: PgPool,
}

impl PostgresGatewayServiceLogStore {
    /// Creates an append adapter over the worker-authorized pool.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl GatewayServiceLogStore for PostgresGatewayServiceLogStore {
    // One transaction owns quota, gateway, instance, and epoch locks so the
    // append cannot publish a partially validated batch or invert lock order.
    #[allow(clippy::too_many_lines)]
    async fn append_batch(
        &self,
        lease: &GatewayServiceInstanceLease,
        owner: &GatewayServiceOwner,
        batch: GatewayServiceLogAppendBatch,
    ) -> Result<GatewayServiceLogAppendOutcome, GatewayServiceLogStoreError> {
        validate_lease(lease)?;
        owner
            .validate()
            .map_err(|_| GatewayServiceLogStoreError::InvalidArgument)?;
        batch.validate()?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;

        // Project identity is read before locking the quota row. The gateway
        // lock below revalidates it, preventing a stale caller from choosing a
        // different project's quota row. All subsequent append/maintenance
        // code must use this quota -> gateway -> instance -> epoch order.
        let project_id: Option<Uuid> = sqlx::query_scalar(
            "SELECT project_id
               FROM gateway_revisions
              WHERE id = $1 AND gateway_id = $2
                AND handler_contract = 'http.service.v1'",
        )
        .bind(lease.identity.revision_id)
        .bind(lease.identity.gateway_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let project_id = project_id.ok_or(GatewayServiceLogStoreError::StaleLease)?;

        sqlx::query(
            "INSERT INTO gateway_service_log_project_usage (project_id)
             VALUES ($1) ON CONFLICT (project_id) DO NOTHING",
        )
        .bind(project_id)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        let mut usage = sqlx::query_as::<_, UsageRow>(
            "SELECT retained_bytes AS bytes, retained_chunks AS chunks,
                    retained_epochs AS epochs
               FROM gateway_service_log_project_usage
              WHERE project_id = $1 FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(GatewayServiceLogStoreError::StaleLease)?;

        let gateway_project: Option<Uuid> =
            sqlx::query_scalar("SELECT project_id FROM gateways WHERE id = $1 FOR UPDATE")
                .bind(lease.identity.gateway_id)
                .fetch_optional(&mut *transaction)
                .await
                .map_err(storage)?;
        if gateway_project != Some(project_id) {
            return Err(GatewayServiceLogStoreError::StaleLease);
        }
        let mode: Option<String> = sqlx::query_scalar(
            "SELECT service_log_capture_mode
               FROM gateway_revisions
              WHERE id = $1 AND gateway_id = $2
                AND project_id = $3 AND handler_contract = 'http.service.v1'",
        )
        .bind(lease.identity.revision_id)
        .bind(lease.identity.gateway_id)
        .bind(project_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        if mode.as_deref() != Some("application") {
            return Err(GatewayServiceLogStoreError::Disabled);
        }

        let current = sqlx::query_as::<_, InstanceRow>(
            "SELECT id, gateway_id, revision_id, owner_host_id, owner_uuid,
                    fencing_token, vm_id, state, lease_expires_at
               FROM gateway_service_instances
              WHERE id = $1 AND gateway_id = $2 AND revision_id = $3
              FOR UPDATE",
        )
        .bind(lease.identity.instance_id)
        .bind(lease.identity.gateway_id)
        .bind(lease.identity.revision_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(GatewayServiceLogStoreError::StaleLease)?;
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?;
        ensure_current_lease(&current, lease, owner, now)?;
        if current.gateway_id != lease.identity.gateway_id
            || current.revision_id != lease.identity.revision_id
        {
            return Err(GatewayServiceLogStoreError::StaleLease);
        }

        let mut instance_usage = sqlx::query_as::<_, InstanceUsageRow>(
            "SELECT coalesce(sum(retained_bytes), 0)::bigint AS bytes,
                    coalesce(sum(retained_chunks), 0)::bigint AS chunks
               FROM gateway_service_log_epochs
              WHERE instance_id = $1",
        )
        .bind(lease.identity.instance_id)
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;

        let epoch = sqlx::query_as::<_, EpochRow>(
            "SELECT acknowledged_through, retained_bytes, retained_chunks,
                    producer_dropped_chunks, producer_dropped_bytes,
                    provider_lagged_events, storage_dropped_chunks,
                    storage_dropped_bytes
               FROM gateway_service_log_epochs
              WHERE instance_id = $1 AND fencing_token = $2
              FOR UPDATE",
        )
        .bind(lease.identity.instance_id)
        .bind(lease.fencing_token)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let mut epoch = if let Some(epoch) = epoch {
            epoch
        } else {
            if usage.epochs >= MAX_PROJECT_EPOCHS {
                let dropped_bytes = batch
                    .records
                    .iter()
                    .map(|record| i64::try_from(record.bytes.len()).unwrap_or(i64::MAX))
                    .fold(0_i64, i64::saturating_add);
                sqlx::query(
                    "UPDATE gateway_service_log_project_usage
                        SET storage_dropped_chunks = CASE
                                WHEN storage_dropped_chunks > 9223372036854775807 - $2
                                THEN 9223372036854775807
                                ELSE storage_dropped_chunks + $2 END,
                            storage_dropped_bytes = CASE
                                WHEN storage_dropped_bytes > 9223372036854775807 - $3
                                THEN 9223372036854775807
                                ELSE storage_dropped_bytes + $3 END,
                            updated_at = now()
                      WHERE project_id = $1",
                )
                .bind(project_id)
                .bind(i64::try_from(batch.records.len()).unwrap_or(i64::MAX))
                .bind(dropped_bytes)
                .execute(&mut *transaction)
                .await
                .map_err(storage)?;
                transaction.commit().await.map_err(storage)?;
                return Err(GatewayServiceLogStoreError::Capacity);
            }
            sqlx::query(
                "INSERT INTO gateway_service_log_epochs
                    (instance_id, gateway_id, revision_id, project_id, fencing_token)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(lease.identity.instance_id)
            .bind(lease.identity.gateway_id)
            .bind(lease.identity.revision_id)
            .bind(project_id)
            .bind(lease.fencing_token)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
            usage.epochs += 1;
            sqlx::query_as::<_, EpochRow>(
                "SELECT acknowledged_through, retained_bytes, retained_chunks,
                        producer_dropped_chunks, producer_dropped_bytes,
                        provider_lagged_events, storage_dropped_chunks,
                        storage_dropped_bytes
                   FROM gateway_service_log_epochs
                  WHERE instance_id = $1 AND fencing_token = $2
                  FOR UPDATE",
            )
            .bind(lease.identity.instance_id)
            .bind(lease.fencing_token)
            .fetch_one(&mut *transaction)
            .await
            .map_err(storage)?
        };

        let mut outcome = GatewayServiceLogAppendOutcome::default();
        let mut acknowledged = epoch.acknowledged_through;
        for record in batch.records {
            let sequence = i64::try_from(record.sequence)
                .map_err(|_| GatewayServiceLogStoreError::InvalidArgument)?;
            let existing = sqlx::query_as::<_, ChunkRow>(
                "SELECT stream, bytes
                   FROM gateway_service_log_chunks
                  WHERE instance_id = $1 AND fencing_token = $2 AND sequence = $3",
            )
            .bind(lease.identity.instance_id)
            .bind(lease.fencing_token)
            .bind(sequence)
            .fetch_optional(&mut *transaction)
            .await
            .map_err(storage)?;
            if let Some(existing) = existing {
                if existing.stream != stream_name(record.stream) || existing.bytes != record.bytes {
                    return Err(GatewayServiceLogStoreError::Conflict);
                }
                outcome.duplicate_chunks = outcome.duplicate_chunks.saturating_add(1);
                acknowledged = acknowledged.max(sequence);
                continue;
            }
            if sequence <= acknowledged {
                // This sequence was already acknowledged and its row may have
                // been evicted by a later maintenance slice. Never reinsert it.
                continue;
            }
            let byte_count = i64::try_from(record.bytes.len())
                .map_err(|_| GatewayServiceLogStoreError::InvalidArgument)?;
            let instance_fits = instance_usage.chunks < MAX_INSTANCE_CHUNKS
                && instance_usage
                    .bytes
                    .checked_add(byte_count)
                    .is_some_and(|value| value <= MAX_INSTANCE_BYTES);
            let project_fits = usage.chunks < MAX_PROJECT_CHUNKS
                && usage
                    .bytes
                    .checked_add(byte_count)
                    .is_some_and(|value| value <= MAX_PROJECT_BYTES);
            acknowledged = acknowledged.max(sequence);
            if !instance_fits || !project_fits {
                outcome.storage_dropped_chunks = outcome.storage_dropped_chunks.saturating_add(1);
                epoch.storage_dropped_chunks = epoch.storage_dropped_chunks.saturating_add(1);
                epoch.storage_dropped_bytes =
                    epoch.storage_dropped_bytes.saturating_add(byte_count);
                continue;
            }
            sqlx::query(
                "INSERT INTO gateway_service_log_chunks
                    (instance_id, gateway_id, revision_id, project_id, fencing_token,
                     sequence, stream, observed_at, bytes)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(lease.identity.instance_id)
            .bind(lease.identity.gateway_id)
            .bind(lease.identity.revision_id)
            .bind(project_id)
            .bind(lease.fencing_token)
            .bind(sequence)
            .bind(stream_name(record.stream))
            .bind(record.observed_at)
            .bind(record.bytes)
            .execute(&mut *transaction)
            .await
            .map_err(storage)?;
            epoch.retained_chunks += 1;
            epoch.retained_bytes += byte_count;
            instance_usage.chunks += 1;
            instance_usage.bytes += byte_count;
            usage.chunks += 1;
            usage.bytes += byte_count;
            outcome.accepted_chunks = outcome.accepted_chunks.saturating_add(1);
        }
        epoch.producer_dropped_chunks = epoch
            .producer_dropped_chunks
            .max(i64::try_from(batch.loss.total_chunks()).unwrap_or(i64::MAX));
        epoch.producer_dropped_bytes = epoch
            .producer_dropped_bytes
            .max(i64::try_from(batch.loss.total_bytes()).unwrap_or(i64::MAX));
        epoch.provider_lagged_events = epoch
            .provider_lagged_events
            .max(i64::try_from(batch.loss.provider_lagged_events).unwrap_or(i64::MAX));
        sqlx::query(
            "UPDATE gateway_service_log_epochs
                SET acknowledged_through = $3, retained_bytes = $4,
                    retained_chunks = $5, producer_dropped_chunks = $6,
                    producer_dropped_bytes = $7, provider_lagged_events = $8,
                    storage_dropped_chunks = $9, storage_dropped_bytes = $10,
                    updated_at = now()
              WHERE instance_id = $1 AND fencing_token = $2",
        )
        .bind(lease.identity.instance_id)
        .bind(lease.fencing_token)
        .bind(acknowledged)
        .bind(epoch.retained_bytes)
        .bind(epoch.retained_chunks)
        .bind(epoch.producer_dropped_chunks)
        .bind(epoch.producer_dropped_bytes)
        .bind(epoch.provider_lagged_events)
        .bind(epoch.storage_dropped_chunks)
        .bind(epoch.storage_dropped_bytes)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_bytes = $2, retained_chunks = $3,
                    retained_epochs = $4, updated_at = now()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .bind(usage.bytes)
        .bind(usage.chunks)
        .bind(usage.epochs)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        transaction.commit().await.map_err(storage)?;
        outcome.acknowledged_through = u64::try_from(acknowledged).ok();
        outcome.retained_instance_bytes = u64::try_from(instance_usage.bytes).unwrap_or(0);
        outcome.retained_instance_chunks = u64::try_from(instance_usage.chunks).unwrap_or(0);
        Ok(outcome)
    }
}

const fn stream_name(stream: LogStream) -> &'static str {
    match stream {
        LogStream::Stdout => "stdout",
        LogStream::Stderr => "stderr",
        _ => "unknown",
    }
}

fn validate_lease(lease: &GatewayServiceInstanceLease) -> Result<(), GatewayServiceLogStoreError> {
    if lease.identity.instance_id.is_nil()
        || lease.identity.gateway_id.is_nil()
        || lease.identity.revision_id.is_nil()
        || lease.owner_uuid.is_nil()
        || lease.fencing_token <= 0
        || lease.vm_id != format!("gateway-service-{}", lease.identity.instance_id)
        || lease.owner_host_id.is_empty()
        || lease.owner_host_id.len() > MAX_SERVICE_OWNER_HOST_BYTES
        || lease.owner_host_id != lease.owner_host_id.trim()
        || lease
            .owner_host_id
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
    {
        return Err(GatewayServiceLogStoreError::InvalidArgument);
    }
    Ok(())
}

fn ensure_current_lease(
    current: &InstanceRow,
    lease: &GatewayServiceInstanceLease,
    owner: &GatewayServiceOwner,
    now: OffsetDateTime,
) -> Result<(), GatewayServiceLogStoreError> {
    if current.id != lease.identity.instance_id
        || current.owner_host_id != lease.owner_host_id
        || current.owner_uuid != lease.owner_uuid
        || current.owner_host_id != owner.host_id
        || current.owner_uuid != owner.owner_uuid
        || current.fencing_token != lease.fencing_token
        || current.vm_id != lease.vm_id
        || current.state == "cleaned"
        || current.lease_expires_at <= now
    {
        return Err(GatewayServiceLogStoreError::StaleLease);
    }
    Ok(())
}

fn storage(_error: sqlx::Error) -> GatewayServiceLogStoreError {
    GatewayServiceLogStoreError::Unavailable
}

#[derive(Debug, FromRow)]
struct UsageRow {
    bytes: i64,
    chunks: i64,
    epochs: i32,
}

#[derive(Debug, FromRow)]
struct EpochRow {
    acknowledged_through: i64,
    retained_bytes: i64,
    retained_chunks: i64,
    producer_dropped_chunks: i64,
    producer_dropped_bytes: i64,
    provider_lagged_events: i64,
    storage_dropped_chunks: i64,
    storage_dropped_bytes: i64,
}

#[derive(Debug, FromRow)]
struct InstanceUsageRow {
    bytes: i64,
    chunks: i64,
}

#[derive(FromRow)]
struct ChunkRow {
    stream: String,
    bytes: Vec<u8>,
}

#[derive(Debug, FromRow)]
struct InstanceRow {
    id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    owner_host_id: String,
    owner_uuid: Uuid,
    fencing_token: i64,
    vm_id: String,
    state: String,
    lease_expires_at: OffsetDateTime,
}
