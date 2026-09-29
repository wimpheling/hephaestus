//! Transactional project-level service-log retention maintenance.

use async_trait::async_trait;
use gateway_domain::{
    GatewayServiceLogMaintenance, GatewayServiceLogMaintenanceError,
    GatewayServiceLogMaintenancePolicy, GatewayServiceLogMaintenanceReport,
};
use std::collections::BTreeSet;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    DeletedChunks, MaintenanceProjectUsage, MaintenanceUsage, PostgresGatewayServiceLogStore,
    RETENTION_HOURS, delete_gc_log_epochs, delete_log_chunks, has_more_maintenance_work,
    load_gc_log_epochs, load_instance_epochs, load_instance_usage, load_log_chunks,
    lock_retention_scopes, maintenance_storage, maintenance_usize, pressure_above_low_instance,
    pressure_above_low_project, pressure_needed_instance, pressure_needed_project,
    same_chunk_identity,
};

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
