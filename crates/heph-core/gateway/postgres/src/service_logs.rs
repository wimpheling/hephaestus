//! Worker-only durable append adapter for opt-in application service logs.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome,
    GatewayServiceLogMaintenance, GatewayServiceLogMaintenanceError,
    GatewayServiceLogMaintenancePolicy, GatewayServiceLogMaintenanceProjectPage,
    GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceProjects,
    GatewayServiceLogMaintenanceReport, GatewayServiceLogStore, GatewayServiceLogStoreError,
    GatewayServiceOwner, MAX_SERVICE_LOG_INSTANCE_BYTES, MAX_SERVICE_LOG_INSTANCE_CHUNKS,
    MAX_SERVICE_LOG_PROJECT_BYTES, MAX_SERVICE_LOG_PROJECT_CHUNKS, MAX_SERVICE_OWNER_HOST_BYTES,
};
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
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

#[async_trait]
impl GatewayServiceLogMaintenanceProjects for PostgresGatewayServiceLogStore {
    async fn list_projects(
        &self,
        page: GatewayServiceLogMaintenanceProjectPage,
    ) -> Result<GatewayServiceLogMaintenanceProjectPageResult, GatewayServiceLogMaintenanceError>
    {
        page.validate()?;
        let mut projects = sqlx::query_scalar::<_, Uuid>(
            "SELECT project_id
               FROM gateway_service_log_project_usage
              WHERE ($1::uuid IS NULL OR project_id > $1)
              ORDER BY project_id
              LIMIT $2",
        )
        .bind(page.after)
        .bind(i64::from(page.limit) + 1)
        .fetch_all(&self.pool)
        .await
        .map_err(maintenance_storage)?;
        let next_after = if projects.len() > usize::from(page.limit) {
            projects.pop();
            projects.last().copied()
        } else {
            None
        };
        Ok(GatewayServiceLogMaintenanceProjectPageResult {
            projects,
            next_after,
        })
    }
}

#[async_trait]
impl GatewayServiceLogMaintenance for PostgresGatewayServiceLogStore {
    // The phases are deliberately kept together so one transaction can gather
    // and lock every affected scope in the published quota order.
    #[allow(clippy::too_many_lines)]
    async fn maintain_project(
        &self,
        project_id: Uuid,
        policy: GatewayServiceLogMaintenancePolicy,
    ) -> Result<GatewayServiceLogMaintenanceReport, GatewayServiceLogMaintenanceError> {
        if project_id.is_nil() || !policy.is_valid() {
            return Err(GatewayServiceLogMaintenanceError::InvalidArgument);
        }
        let mut transaction = self.pool.begin().await.map_err(maintenance_storage)?;
        let now: OffsetDateTime = sqlx::query_scalar("SELECT clock_timestamp()")
            .fetch_one(&mut *transaction)
            .await
            .map_err(maintenance_storage)?;
        let Some(mut usage) = sqlx::query_as::<_, MaintenanceProjectUsage>(
            "SELECT retained_bytes AS bytes, retained_chunks AS chunks,
                    retained_epochs AS epochs, pressure_cleanup_pending
               FROM gateway_service_log_project_usage
              WHERE project_id = $1
              FOR UPDATE",
        )
        .bind(project_id)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(maintenance_storage)?
        else {
            transaction.commit().await.map_err(maintenance_storage)?;
            return Ok(GatewayServiceLogMaintenanceReport::default());
        };

        let cutoff = now - time::Duration::hours(RETENTION_HOURS);
        let mut report = GatewayServiceLogMaintenanceReport::default();
        let remaining_chunks = policy.max_chunks;
        let expired = load_log_chunks(
            &mut transaction,
            project_id,
            Some(cutoff),
            None,
            remaining_chunks,
        )
        .await?;
        let mut instance_usage = load_instance_usage(&mut transaction, project_id).await?;
        let expired_totals = expired
            .iter()
            .fold(DeletedChunks::default(), |mut total, chunk| {
                total.chunks = total.chunks.saturating_add(1);
                total.bytes = total.bytes.saturating_add(chunk.bytes);
                total
            });
        let pressure_usage = MaintenanceUsage {
            bytes: usage.bytes.saturating_sub(expired_totals.bytes),
            chunks: usage.chunks.saturating_sub(expired_totals.chunks),
            pressure_cleanup_pending: usage.pressure_cleanup_pending,
        };
        let mut pressure_instances = instance_usage.clone();
        for chunk in &expired {
            if let Some(instance) = pressure_instances.get_mut(&chunk.instance_id) {
                instance.bytes = instance.bytes.saturating_sub(chunk.bytes);
                instance.chunks = instance.chunks.saturating_sub(1);
            }
        }
        let project_pressured = pressure_usage.pressure_cleanup_pending
            || pressure_needed_project(pressure_usage.bytes, pressure_usage.chunks);
        usage.pressure_cleanup_pending |= project_pressured;
        for instance in pressure_instances.values_mut() {
            instance.pressure_cleanup_pending |=
                pressure_needed_instance(instance.bytes, instance.chunks);
        }
        let affected_instance_ids = pressure_instances
            .iter()
            .filter_map(|(instance_id, instance)| {
                instance.pressure_cleanup_pending.then_some(*instance_id)
            })
            .collect::<BTreeSet<_>>();
        let mut selected_pressure = Vec::new();
        let remaining_after_expiry = remaining_chunks.saturating_sub(expired.len());
        if remaining_after_expiry > 0 {
            let pressured_instance_ids = pressure_instances
                .iter()
                .filter_map(|(instance_id, usage)| {
                    (usage.pressure_cleanup_pending
                        || pressure_needed_instance(usage.bytes, usage.chunks))
                    .then_some(*instance_id)
                })
                .collect::<BTreeSet<_>>();
            if project_pressured || !pressured_instance_ids.is_empty() {
                let filter = (!project_pressured).then_some(pressured_instance_ids.clone());
                let candidates = load_log_chunks(
                    &mut transaction,
                    project_id,
                    None,
                    filter.as_ref(),
                    remaining_after_expiry.saturating_add(expired.len()),
                )
                .await?;
                if !candidates.is_empty() {
                    let mut selected = Vec::new();
                    let mut projected_usage = pressure_usage;
                    let mut projected_instances = pressure_instances.clone();
                    for candidate in candidates {
                        if expired
                            .iter()
                            .any(|expired_chunk| same_chunk_identity(expired_chunk, &candidate))
                        {
                            continue;
                        }
                        let Some(instance) = projected_instances.get(&candidate.instance_id) else {
                            continue;
                        };
                        let project_cleanup = project_pressured
                            && pressure_above_low_project(
                                projected_usage.bytes,
                                projected_usage.chunks,
                            );
                        let instance_cleanup = instance.pressure_cleanup_pending
                            && pressure_above_low_instance(instance.bytes, instance.chunks);
                        if !project_cleanup && !instance_cleanup {
                            continue;
                        }
                        selected.push(candidate.clone());
                        let bytes = candidate.bytes;
                        let projected_bytes = projected_usage.bytes.saturating_sub(bytes);
                        let projected_chunks = projected_usage.chunks.saturating_sub(1);
                        if let Some(instance) = projected_instances.get_mut(&candidate.instance_id)
                        {
                            instance.bytes = instance.bytes.saturating_sub(bytes);
                            instance.chunks = instance.chunks.saturating_sub(1);
                        }
                        projected_usage.bytes = projected_bytes;
                        projected_usage.chunks = projected_chunks;
                        if selected.len() >= remaining_after_expiry
                            || ((!project_pressured
                                || !pressure_above_low_project(
                                    projected_usage.bytes,
                                    projected_usage.chunks,
                                ))
                                && projected_instances.values().all(|value| {
                                    !value.pressure_cleanup_pending
                                        || !pressure_above_low_instance(value.bytes, value.chunks)
                                }))
                        {
                            break;
                        }
                    }
                    if !selected.is_empty() {
                        selected_pressure = selected;
                    }
                }
            }
        }

        let gc_candidates =
            load_gc_log_epochs(&mut transaction, project_id, cutoff, policy.max_epochs).await?;
        let affected_epochs =
            load_instance_epochs(&mut transaction, project_id, &affected_instance_ids).await?;
        let mut lock_chunks = expired.clone();
        lock_chunks.extend(selected_pressure.iter().cloned());
        let mut lock_epochs = gc_candidates.clone();
        lock_epochs.extend(affected_epochs);
        lock_retention_scopes(&mut transaction, &lock_chunks, &lock_epochs).await?;
        if !expired.is_empty() {
            let deleted = delete_log_chunks(&mut transaction, &expired, false).await?;
            report.expired_chunks = maintenance_usize(deleted.chunks);
            report.expired_bytes = maintenance_usize(deleted.bytes);
            usage.bytes = usage.bytes.saturating_sub(deleted.bytes);
            usage.chunks = usage.chunks.saturating_sub(deleted.chunks);
            instance_usage = load_instance_usage(&mut transaction, project_id).await?;
        }
        if !selected_pressure.is_empty() {
            let deleted = delete_log_chunks(&mut transaction, &selected_pressure, true).await?;
            report.evicted_chunks = maintenance_usize(deleted.chunks);
            report.evicted_bytes = maintenance_usize(deleted.bytes);
            usage.bytes = usage.bytes.saturating_sub(deleted.bytes);
            usage.chunks = usage.chunks.saturating_sub(deleted.chunks);
            instance_usage = load_instance_usage(&mut transaction, project_id).await?;
        }

        usage.pressure_cleanup_pending =
            usage.pressure_cleanup_pending && pressure_above_low_project(usage.bytes, usage.chunks);
        for (instance_id, instance) in &mut instance_usage {
            let was_pressure_pending = pressure_instances
                .get(instance_id)
                .is_some_and(|value| value.pressure_cleanup_pending);
            instance.pressure_cleanup_pending = was_pressure_pending
                && pressure_above_low_instance(instance.bytes, instance.chunks);
        }
        for instance_id in affected_instance_ids {
            let instance = instance_usage
                .get(&instance_id)
                .expect("affected instance usage remains present");
            sqlx::query(
                "UPDATE gateway_service_log_epochs
                    SET pressure_cleanup_pending = $3,
                        updated_at = updated_at
                  WHERE project_id = $1 AND instance_id = $2",
            )
            .bind(project_id)
            .bind(instance_id)
            .bind(instance.pressure_cleanup_pending)
            .execute(&mut *transaction)
            .await
            .map_err(maintenance_storage)?;
        }
        let project_pressure_pending = usage.pressure_cleanup_pending;
        sqlx::query(
            "UPDATE gateway_service_log_project_usage
                SET retained_bytes = GREATEST(0, $2),
                    retained_chunks = GREATEST(0, $3),
                    pressure_cleanup_pending = $4,
                    updated_at = clock_timestamp()
              WHERE project_id = $1",
        )
        .bind(project_id)
        .bind(usage.bytes)
        .bind(usage.chunks)
        .bind(project_pressure_pending)
        .execute(&mut *transaction)
        .await
        .map_err(maintenance_storage)?;

        let metadata = delete_gc_log_epochs(&mut transaction, &gc_candidates).await?;
        report.metadata_epochs = metadata;
        if metadata > 0 {
            usage.epochs = usage
                .epochs
                .saturating_sub(i32::try_from(metadata).unwrap_or(i32::MAX));
            sqlx::query(
                "UPDATE gateway_service_log_project_usage
                    SET retained_epochs = GREATEST(0, $2),
                        updated_at = clock_timestamp()
                  WHERE project_id = $1",
            )
            .bind(project_id)
            .bind(usage.epochs)
            .execute(&mut *transaction)
            .await
            .map_err(maintenance_storage)?;
        }

        report.has_more = has_more_maintenance_work(
            &mut transaction,
            project_id,
            cutoff,
            usage.bytes,
            usage.chunks,
            project_pressure_pending,
            &instance_usage,
            policy.max_epochs,
        )
        .await?;
        transaction.commit().await.map_err(maintenance_storage)?;
        Ok(report)
    }
}

#[derive(Debug, Clone, FromRow)]
struct MaintenanceChunk {
    instance_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    project_id: Uuid,
    fencing_token: i64,
    sequence: i64,
    bytes: i64,
}

#[derive(Debug, Clone, FromRow)]
struct MaintenanceEpoch {
    instance_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
    project_id: Uuid,
    fencing_token: i64,
}

#[derive(Debug, Default, Clone, Copy)]
struct MaintenanceUsage {
    bytes: i64,
    chunks: i64,
    pressure_cleanup_pending: bool,
}

#[derive(Debug, FromRow)]
struct MaintenanceProjectUsage {
    bytes: i64,
    chunks: i64,
    epochs: i32,
    pressure_cleanup_pending: bool,
}

#[derive(Debug, Default, Clone, Copy)]
struct DeletedChunks {
    chunks: i64,
    bytes: i64,
}

async fn load_log_chunks(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    cutoff: Option<OffsetDateTime>,
    instance_filter: Option<&BTreeSet<Uuid>>,
    limit: usize,
) -> Result<Vec<MaintenanceChunk>, GatewayServiceLogMaintenanceError> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let rows = if let Some(instance_filter) = instance_filter {
        let instances = instance_filter.iter().copied().collect::<Vec<_>>();
        if let Some(cutoff) = cutoff {
            sqlx::query_as::<_, MaintenanceChunk>(
                "SELECT instance_id, gateway_id, revision_id, project_id,
                        fencing_token, sequence, octet_length(bytes)::bigint AS bytes
                   FROM gateway_service_log_chunks
                  WHERE project_id = $1 AND stored_at < $2
                    AND instance_id = ANY($3)
                  ORDER BY stored_at, instance_id, fencing_token, sequence
                  LIMIT $4",
            )
            .bind(project_id)
            .bind(cutoff)
            .bind(instances)
            .bind(limit)
            .fetch_all(&mut **transaction)
            .await
        } else {
            sqlx::query_as::<_, MaintenanceChunk>(
                "SELECT instance_id, gateway_id, revision_id, project_id,
                        fencing_token, sequence, octet_length(bytes)::bigint AS bytes
                   FROM gateway_service_log_chunks
                  WHERE project_id = $1 AND instance_id = ANY($2)
                  ORDER BY stored_at, instance_id, fencing_token, sequence
                  LIMIT $3",
            )
            .bind(project_id)
            .bind(instances)
            .bind(limit)
            .fetch_all(&mut **transaction)
            .await
        }
    } else if let Some(cutoff) = cutoff {
        sqlx::query_as::<_, MaintenanceChunk>(
            "SELECT instance_id, gateway_id, revision_id, project_id,
                        fencing_token, sequence, octet_length(bytes)::bigint AS bytes
                   FROM gateway_service_log_chunks
                  WHERE project_id = $1 AND stored_at < $2
                  ORDER BY stored_at, instance_id, fencing_token, sequence
                  LIMIT $3",
        )
        .bind(project_id)
        .bind(cutoff)
        .bind(limit)
        .fetch_all(&mut **transaction)
        .await
    } else {
        sqlx::query_as::<_, MaintenanceChunk>(
            "SELECT instance_id, gateway_id, revision_id, project_id,
                        fencing_token, sequence, octet_length(bytes)::bigint AS bytes
                   FROM gateway_service_log_chunks
                  WHERE project_id = $1
                  ORDER BY stored_at, instance_id, fencing_token, sequence
                  LIMIT $2",
        )
        .bind(project_id)
        .bind(limit)
        .fetch_all(&mut **transaction)
        .await
    };
    rows.map_err(maintenance_storage)
}

async fn load_instance_usage(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
) -> Result<BTreeMap<Uuid, MaintenanceUsage>, GatewayServiceLogMaintenanceError> {
    let rows = sqlx::query_as::<_, (Uuid, i64, i64, bool)>(
        "SELECT instance_id,
                coalesce(sum(retained_bytes), 0)::bigint,
                coalesce(sum(retained_chunks), 0)::bigint,
                bool_or(pressure_cleanup_pending)
           FROM gateway_service_log_epochs
          WHERE project_id = $1
          GROUP BY instance_id",
    )
    .bind(project_id)
    .fetch_all(&mut **transaction)
    .await
    .map_err(maintenance_storage)?;
    Ok(rows
        .into_iter()
        .map(|(instance_id, bytes, chunks, pressure_cleanup_pending)| {
            (
                instance_id,
                MaintenanceUsage {
                    bytes,
                    chunks,
                    pressure_cleanup_pending,
                },
            )
        })
        .collect())
}

async fn load_instance_epochs(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    instance_ids: &BTreeSet<Uuid>,
) -> Result<Vec<MaintenanceEpoch>, GatewayServiceLogMaintenanceError> {
    if instance_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, MaintenanceEpoch>(
        "SELECT instance_id, gateway_id, revision_id, project_id, fencing_token
           FROM gateway_service_log_epochs
          WHERE project_id = $1 AND instance_id = ANY($2)
          ORDER BY gateway_id, instance_id, fencing_token",
    )
    .bind(project_id)
    .bind(instance_ids.iter().copied().collect::<Vec<_>>())
    .fetch_all(&mut **transaction)
    .await
    .map_err(maintenance_storage)
}

fn same_chunk_identity(left: &MaintenanceChunk, right: &MaintenanceChunk) -> bool {
    left.instance_id == right.instance_id
        && left.gateway_id == right.gateway_id
        && left.revision_id == right.revision_id
        && left.project_id == right.project_id
        && left.fencing_token == right.fencing_token
        && left.sequence == right.sequence
}

async fn lock_retention_scopes(
    transaction: &mut Transaction<'_, Postgres>,
    chunks: &[MaintenanceChunk],
    epochs: &[MaintenanceEpoch],
) -> Result<(), GatewayServiceLogMaintenanceError> {
    let gateways = chunks
        .iter()
        .map(|chunk| chunk.gateway_id)
        .chain(epochs.iter().map(|epoch| epoch.gateway_id))
        .collect::<BTreeSet<_>>();
    for gateway_id in gateways {
        sqlx::query("SELECT id FROM gateways WHERE id = $1 FOR UPDATE")
            .bind(gateway_id)
            .execute(&mut **transaction)
            .await
            .map_err(maintenance_storage)?;
    }
    let instances = chunks
        .iter()
        .map(|chunk| chunk.instance_id)
        .chain(epochs.iter().map(|epoch| epoch.instance_id))
        .collect::<BTreeSet<_>>();
    for instance_id in instances {
        sqlx::query("SELECT id FROM gateway_service_instances WHERE id = $1 FOR UPDATE")
            .bind(instance_id)
            .execute(&mut **transaction)
            .await
            .map_err(maintenance_storage)?;
    }
    let epochs = chunks
        .iter()
        .map(|chunk| (chunk.instance_id, chunk.fencing_token))
        .chain(
            epochs
                .iter()
                .map(|epoch| (epoch.instance_id, epoch.fencing_token)),
        )
        .collect::<BTreeSet<_>>();
    for (instance_id, fencing_token) in epochs {
        sqlx::query(
            "SELECT instance_id
               FROM gateway_service_log_epochs
              WHERE instance_id = $1 AND fencing_token = $2
              FOR UPDATE",
        )
        .bind(instance_id)
        .bind(fencing_token)
        .execute(&mut **transaction)
        .await
        .map_err(maintenance_storage)?;
    }
    Ok(())
}

async fn delete_log_chunks(
    transaction: &mut Transaction<'_, Postgres>,
    chunks: &[MaintenanceChunk],
    pressure: bool,
) -> Result<DeletedChunks, GatewayServiceLogMaintenanceError> {
    let mut deltas = BTreeMap::<(Uuid, Uuid, Uuid, Uuid, i64), DeletedChunks>::new();
    for chunk in chunks {
        let deleted = sqlx::query_as::<_, (i64,)>(
            "DELETE FROM gateway_service_log_chunks
              WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
                AND project_id = $4 AND fencing_token = $5 AND sequence = $6
              RETURNING octet_length(bytes)::bigint",
        )
        .bind(chunk.instance_id)
        .bind(chunk.gateway_id)
        .bind(chunk.revision_id)
        .bind(chunk.project_id)
        .bind(chunk.fencing_token)
        .bind(chunk.sequence)
        .fetch_optional(&mut **transaction)
        .await
        .map_err(maintenance_storage)?;
        if let Some((bytes,)) = deleted {
            let key = (
                chunk.instance_id,
                chunk.gateway_id,
                chunk.revision_id,
                chunk.project_id,
                chunk.fencing_token,
            );
            let delta = deltas.entry(key).or_default();
            delta.chunks = delta.chunks.saturating_add(1);
            delta.bytes = delta.bytes.saturating_add(bytes);
        }
    }
    for ((instance_id, gateway_id, revision_id, project_id, fencing_token), delta) in &deltas {
        sqlx::query(
            "UPDATE gateway_service_log_epochs
                SET retained_bytes = GREATEST(0, retained_bytes - $6),
                    retained_chunks = GREATEST(0, retained_chunks - $7),
                    evicted_bytes = CASE
                        WHEN $8 AND evicted_bytes > 9223372036854775807 - $6
                            THEN 9223372036854775807
                        WHEN $8 THEN evicted_bytes + $6
                        ELSE evicted_bytes
                    END,
                    evicted_chunks = CASE
                        WHEN $8 AND evicted_chunks > 9223372036854775807 - $7
                            THEN 9223372036854775807
                        WHEN $8 THEN evicted_chunks + $7
                        ELSE evicted_chunks
                    END,
                    updated_at = clock_timestamp()
              WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
                AND project_id = $4 AND fencing_token = $5",
        )
        .bind(instance_id)
        .bind(gateway_id)
        .bind(revision_id)
        .bind(project_id)
        .bind(fencing_token)
        .bind(delta.bytes)
        .bind(delta.chunks)
        .bind(pressure)
        .execute(&mut **transaction)
        .await
        .map_err(maintenance_storage)?;
    }
    Ok(deltas
        .values()
        .fold(DeletedChunks::default(), |mut total, delta| {
            total.chunks = total.chunks.saturating_add(delta.chunks);
            total.bytes = total.bytes.saturating_add(delta.bytes);
            total
        }))
}

async fn load_gc_log_epochs(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    cutoff: OffsetDateTime,
    limit: usize,
) -> Result<Vec<MaintenanceEpoch>, GatewayServiceLogMaintenanceError> {
    let limit = i64::try_from(limit).unwrap_or(i64::MAX);
    let candidates = sqlx::query_as::<_, MaintenanceEpoch>(
        "SELECT e.instance_id, e.gateway_id, e.revision_id, e.project_id,
                e.fencing_token
           FROM gateway_service_log_epochs e
           JOIN gateway_service_instances i ON i.id = e.instance_id
          WHERE e.project_id = $1
            AND e.retained_bytes = 0 AND e.retained_chunks = 0
            AND e.updated_at < $2
            AND (i.state = 'cleaned' OR i.fencing_token > e.fencing_token)
          ORDER BY e.updated_at, e.instance_id, e.fencing_token
          LIMIT $3",
    )
    .bind(project_id)
    .bind(cutoff)
    .bind(limit)
    .fetch_all(&mut **transaction)
    .await
    .map_err(maintenance_storage)?;
    Ok(candidates)
}

async fn delete_gc_log_epochs(
    transaction: &mut Transaction<'_, Postgres>,
    candidates: &[MaintenanceEpoch],
) -> Result<usize, GatewayServiceLogMaintenanceError> {
    let mut removed = 0;
    for epoch in candidates {
        let result = sqlx::query(
            "DELETE FROM gateway_service_log_epochs e
              USING gateway_service_instances i
              WHERE e.instance_id = $1 AND e.gateway_id = $2 AND e.revision_id = $3
                AND e.project_id = $4 AND e.fencing_token = $5
                AND e.retained_bytes = 0 AND e.retained_chunks = 0
                AND i.id = e.instance_id
                AND (i.state = 'cleaned' OR i.fencing_token > e.fencing_token)",
        )
        .bind(epoch.instance_id)
        .bind(epoch.gateway_id)
        .bind(epoch.revision_id)
        .bind(epoch.project_id)
        .bind(epoch.fencing_token)
        .execute(&mut **transaction)
        .await
        .map_err(maintenance_storage)?;
        removed += usize::try_from(result.rows_affected()).unwrap_or(0);
    }
    Ok(removed)
}

// The arguments are the post-pass counters used to answer `has_more`; keeping
// this as a small pure database helper avoids exposing maintenance state.
#[allow(clippy::too_many_arguments)]
async fn has_more_maintenance_work(
    transaction: &mut Transaction<'_, Postgres>,
    project_id: Uuid,
    cutoff: OffsetDateTime,
    retained_bytes: i64,
    retained_chunks: i64,
    project_pressure_pending: bool,
    instance_usage: &BTreeMap<Uuid, MaintenanceUsage>,
    epoch_limit: usize,
) -> Result<bool, GatewayServiceLogMaintenanceError> {
    if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM gateway_service_log_chunks
              WHERE project_id = $1 AND stored_at < $2
         )",
    )
    .bind(project_id)
    .bind(cutoff)
    .fetch_one(&mut **transaction)
    .await
    .map_err(maintenance_storage)?
    {
        return Ok(true);
    }
    if project_pressure_pending
        || pressure_needed_project(retained_bytes, retained_chunks)
        || instance_usage
            .values()
            .any(|usage| usage.pressure_cleanup_pending)
    {
        return Ok(true);
    }
    let epoch_limit = i64::try_from(epoch_limit).unwrap_or(i64::MAX);
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1
               FROM gateway_service_log_epochs e
               JOIN gateway_service_instances i ON i.id = e.instance_id
              WHERE e.project_id = $1
                AND e.retained_bytes = 0 AND e.retained_chunks = 0
                AND e.updated_at < $2
                AND (i.state = 'cleaned' OR i.fencing_token > e.fencing_token)
              LIMIT $3
         )",
    )
    .bind(project_id)
    .bind(cutoff)
    .bind(epoch_limit)
    .fetch_one(&mut **transaction)
    .await
    .map_err(maintenance_storage)
}

const fn pressure_needed_instance(bytes: i64, chunks: i64) -> bool {
    bytes >= pressure_high(MAX_INSTANCE_BYTES) || chunks >= pressure_high(MAX_INSTANCE_CHUNKS)
}

const fn pressure_needed_project(bytes: i64, chunks: i64) -> bool {
    bytes >= pressure_high(MAX_PROJECT_BYTES) || chunks >= pressure_high(MAX_PROJECT_CHUNKS)
}

const fn pressure_above_low_instance(bytes: i64, chunks: i64) -> bool {
    bytes > pressure_low(MAX_INSTANCE_BYTES) || chunks > pressure_low(MAX_INSTANCE_CHUNKS)
}

const fn pressure_above_low_project(bytes: i64, chunks: i64) -> bool {
    bytes > pressure_low(MAX_PROJECT_BYTES) || chunks > pressure_low(MAX_PROJECT_CHUNKS)
}

const fn pressure_high(cap: i64) -> i64 {
    cap * PRESSURE_HIGH_NUMERATOR / PRESSURE_HIGH_DENOMINATOR
}

const fn pressure_low(cap: i64) -> i64 {
    cap * PRESSURE_LOW_NUMERATOR / PRESSURE_LOW_DENOMINATOR
}

fn maintenance_storage(_error: sqlx::Error) -> GatewayServiceLogMaintenanceError {
    GatewayServiceLogMaintenanceError::Unavailable
}

fn maintenance_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
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
