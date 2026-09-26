//! Deletion, garbage collection, and pressure helpers for service logs.

use gateway_domain::GatewayServiceLogMaintenanceError;
use sqlx::{Postgres, Transaction};
use std::collections::BTreeMap;
use time::OffsetDateTime;
use uuid::Uuid;

use super::{
    DeletedChunks, MAX_INSTANCE_BYTES, MAX_INSTANCE_CHUNKS, MAX_PROJECT_BYTES, MAX_PROJECT_CHUNKS,
    MaintenanceChunk, MaintenanceEpoch, MaintenanceUsage, PRESSURE_HIGH_DENOMINATOR,
    PRESSURE_HIGH_NUMERATOR, PRESSURE_LOW_DENOMINATOR, PRESSURE_LOW_NUMERATOR,
};

pub async fn delete_log_chunks(
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

pub async fn load_gc_log_epochs(
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

pub async fn delete_gc_log_epochs(
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
pub async fn has_more_maintenance_work(
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

pub const fn pressure_needed_instance(bytes: i64, chunks: i64) -> bool {
    bytes >= pressure_high(MAX_INSTANCE_BYTES) || chunks >= pressure_high(MAX_INSTANCE_CHUNKS)
}

pub const fn pressure_needed_project(bytes: i64, chunks: i64) -> bool {
    bytes >= pressure_high(MAX_PROJECT_BYTES) || chunks >= pressure_high(MAX_PROJECT_CHUNKS)
}

pub const fn pressure_above_low_instance(bytes: i64, chunks: i64) -> bool {
    bytes > pressure_low(MAX_INSTANCE_BYTES) || chunks > pressure_low(MAX_INSTANCE_CHUNKS)
}

pub const fn pressure_above_low_project(bytes: i64, chunks: i64) -> bool {
    bytes > pressure_low(MAX_PROJECT_BYTES) || chunks > pressure_low(MAX_PROJECT_CHUNKS)
}

const fn pressure_high(cap: i64) -> i64 {
    cap * PRESSURE_HIGH_NUMERATOR / PRESSURE_HIGH_DENOMINATOR
}

const fn pressure_low(cap: i64) -> i64 {
    cap * PRESSURE_LOW_NUMERATOR / PRESSURE_LOW_DENOMINATOR
}

pub fn maintenance_storage(_error: sqlx::Error) -> GatewayServiceLogMaintenanceError {
    GatewayServiceLogMaintenanceError::Unavailable
}

pub fn maintenance_usize(value: i64) -> usize {
    usize::try_from(value).unwrap_or(usize::MAX)
}
