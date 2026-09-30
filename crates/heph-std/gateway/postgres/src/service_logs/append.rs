//! Atomic service-log append implementation.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceInstanceLease, GatewayServiceLogAppendBatch, GatewayServiceLogAppendOutcome,
    GatewayServiceLogStore, GatewayServiceLogStoreError, GatewayServiceOwner,
};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    ChunkRow, EpochRow, InstanceRow, InstanceUsageRow, MAX_INSTANCE_BYTES, MAX_INSTANCE_CHUNKS,
    MAX_PROJECT_BYTES, MAX_PROJECT_CHUNKS, MAX_PROJECT_EPOCHS, PostgresGatewayServiceLogStore,
    UsageRow, ensure_current_lease, storage, stream_name, validate_lease,
};

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
