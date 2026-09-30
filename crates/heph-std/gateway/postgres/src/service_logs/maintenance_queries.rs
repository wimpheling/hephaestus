//! Chunk, epoch, and retention-scope queries for service-log maintenance.

use gateway_domain::GatewayServiceLogMaintenanceError;
use sqlx::{FromRow, Postgres, Transaction};
use std::collections::{BTreeMap, BTreeSet};
use time::OffsetDateTime;
use uuid::Uuid;

use super::maintenance_storage;

#[derive(Debug, Clone, FromRow)]
pub struct MaintenanceChunk {
    pub instance_id: Uuid,
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
    pub project_id: Uuid,
    pub fencing_token: i64,
    pub sequence: i64,
    pub bytes: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct MaintenanceEpoch {
    pub instance_id: Uuid,
    pub gateway_id: Uuid,
    pub revision_id: Uuid,
    pub project_id: Uuid,
    pub fencing_token: i64,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct MaintenanceUsage {
    pub bytes: i64,
    pub chunks: i64,
    pub pressure_cleanup_pending: bool,
}

#[derive(Debug, FromRow)]
pub struct MaintenanceProjectUsage {
    pub bytes: i64,
    pub chunks: i64,
    pub epochs: i32,
    pub pressure_cleanup_pending: bool,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct DeletedChunks {
    pub chunks: i64,
    pub bytes: i64,
}

pub async fn load_log_chunks(
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

pub async fn load_instance_usage(
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

pub async fn load_instance_epochs(
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

pub fn same_chunk_identity(left: &MaintenanceChunk, right: &MaintenanceChunk) -> bool {
    left.instance_id == right.instance_id
        && left.gateway_id == right.gateway_id
        && left.revision_id == right.revision_id
        && left.project_id == right.project_id
        && left.fencing_token == right.fencing_token
        && left.sequence == right.sequence
}

pub async fn lock_retention_scopes(
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
