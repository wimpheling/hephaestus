use crate::{
    rows::{MaterializationJobRow, MaterializedRootRow, ProductionInputRow, ProductionJobRow},
    support::{
        append_event, lease_seconds, parse_digest, parse_path, parse_reference, storage,
        validate_output, validate_reason,
    },
};
use async_trait::async_trait;
use builder_catalog_domain::OciImageId;
use heph_build::{
    ClaimedMaterializationJob, ClaimedProductionJob, MaterializedRoot, OciImageProductionJobStore,
    OciImageProductionOutput, OciWorkerStoreError, RepositoryOciImageProvenance,
};
use sqlx::PgPool;
use std::{path::Path, time::Duration};
use uuid::Uuid;

/// `PostgreSQL` implementation of the OCI production durable-job boundary.
#[derive(Clone)]
pub struct PgOciImageProductionJobStore {
    pool: PgPool,
}

impl PgOciImageProductionJobStore {
    /// Creates the worker adapter using a connection pool authenticated as the
    /// dedicated `hephaestus_worker` role.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl OciImageProductionJobStore for PgOciImageProductionJobStore {
    async fn claim_production(
        &self,
        _worker_name: &str,
        lease: Duration,
    ) -> Result<Option<ClaimedProductionJob>, OciWorkerStoreError> {
        let lease_seconds = lease_seconds(lease)?;
        let row = sqlx::query_as::<_, ProductionJobRow>(
            "WITH candidate AS (
                SELECT job.id
                  FROM repository_oci_image_production_jobs AS job
                 WHERE job.state = 'queued'
                    OR (job.state = 'claimed' AND job.lease_expires_at <= now())
                 ORDER BY job.created_at, job.id
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             UPDATE repository_oci_image_production_jobs AS job
                SET state = 'claimed',
                    lease_expires_at = now() + make_interval(secs => $1),
                    updated_at = now()
               FROM candidate
              WHERE job.id = candidate.id
             RETURNING job.id, job.definition_id",
        )
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        let Some(row) = row else {
            return Ok(None);
        };
        let input = sqlx::query_as::<_, ProductionInputRow>(
            "SELECT definition.source_repository_id, definition.source_revision,
                    definition.project_id,
                    definition.context_digest, definition.dockerfile_path,
                    definition.context_path, definition.base_image_reference
               FROM repository_oci_image_definitions AS definition
              WHERE definition.id = $1 AND definition.status = 'producing'",
        )
        .bind(row.definition_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(OciWorkerStoreError::Conflict)?;
        Ok(Some(ClaimedProductionJob {
            id: row.id,
            project_id: input.project_id,
            image_id: OciImageId::from_uuid(row.definition_id),
            repository_id: input.source_repository_id,
            source_revision: input.source_revision,
            context_digest: parse_digest(input.context_digest)?,
            dockerfile_path: parse_path(input.dockerfile_path)?,
            context_path: parse_path(input.context_path)?,
            base_reference: parse_reference(input.base_image_reference)?,
        }))
    }

    async fn complete_production(
        &self,
        job_id: Uuid,
        materialization_worker_name: &str,
        output: &OciImageProductionOutput,
        provenance: RepositoryOciImageProvenance,
    ) -> Result<(), OciWorkerStoreError> {
        validate_output(output, &provenance)?;
        if materialization_worker_name.trim().is_empty() || materialization_worker_name.len() > 200
        {
            return Err(OciWorkerStoreError::Conflict);
        }
        let provenance = serde_json::to_value(&provenance).map_err(storage)?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let claimed = sqlx::query_as::<_, ProductionJobRow>(
            "UPDATE repository_oci_image_production_jobs
                SET state = 'succeeded', lease_expires_at = NULL,
                    output_image_reference = $2, output_image_digest = $3,
                    provenance = $4, scan_reference = $5,
                    failure_reason = NULL, updated_at = now()
              WHERE id = $1 AND state = 'claimed'
              RETURNING id, definition_id",
        )
        .bind(job_id)
        .bind(output.image_reference.as_str())
        .bind(output.image_digest.as_str())
        .bind(provenance.clone())
        .bind(&output.scan_reference)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(OciWorkerStoreError::Conflict)?;
        let approved: bool = sqlx::query_scalar(
            "SELECT EXISTS (
                SELECT 1
                  FROM registry_publications AS publication
                  JOIN registry_namespaces AS namespace
                    ON namespace.id = publication.namespace_id
                  JOIN repository_oci_image_definitions AS definition
                    ON definition.id = $1
                 WHERE publication.owner_kind = 'repository_oci_image'
                   AND publication.owner_id = definition.id
                   AND publication.project_id = definition.project_id
                   AND publication.state = 'approved'
                   AND publication.expected_digest = $2
                   AND namespace.repository_path = 'projects/' || definition.project_id::text
                       || '/repository-images/' || definition.id::text
                   AND publication.registry_authority || '/' || namespace.repository_path
                       || '@' || publication.expected_digest = $3
             )",
        )
        .bind(claimed.definition_id)
        .bind(output.image_digest.as_str())
        .bind(output.image_reference.as_str())
        .fetch_one(&mut *transaction)
        .await
        .map_err(storage)?;
        if !approved {
            return Err(OciWorkerStoreError::Conflict);
        }
        let updated = sqlx::query(
            "UPDATE repository_oci_image_definitions
                SET status = 'ready', image_reference = $2,
                    image_digest = $3, provenance = $4,
                    failure_reason = NULL, updated_at = now()
              WHERE id = $1 AND status = 'producing'",
        )
        .bind(claimed.definition_id)
        .bind(output.image_reference.as_str())
        .bind(output.image_digest.as_str())
        .bind(provenance)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if updated.rows_affected() != 1 {
            return Err(OciWorkerStoreError::Conflict);
        }
        sqlx::query(
            "INSERT INTO oci_image_materialization_jobs
                (id, worker_name, image_reference, state)
             VALUES (gen_random_uuid(), $1, $2, 'queued')
             ON CONFLICT (worker_name, image_reference) DO UPDATE
                SET state = 'queued', lease_expires_at = NULL, root_path = NULL,
                    failure_reason = NULL, updated_at = now()",
        )
        .bind(materialization_worker_name)
        .bind(output.image_reference.as_str())
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        append_event(
            &mut transaction,
            claimed.definition_id,
            "state_changed",
            "ready",
        )
        .await?;
        transaction.commit().await.map_err(storage)
    }

    async fn fail_production(&self, job_id: Uuid, reason: &str) -> Result<(), OciWorkerStoreError> {
        validate_reason(reason)?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let claimed = sqlx::query_as::<_, ProductionJobRow>(
            "UPDATE repository_oci_image_production_jobs
                SET state = 'failed', lease_expires_at = NULL, failure_reason = $2,
                    updated_at = now()
              WHERE id = $1 AND state = 'claimed'
              RETURNING id, definition_id",
        )
        .bind(job_id)
        .bind(reason)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(OciWorkerStoreError::Conflict)?;
        let updated = sqlx::query(
            "UPDATE repository_oci_image_definitions
                SET status = 'failed', failure_reason = $2, updated_at = now()
              WHERE id = $1 AND status = 'producing'",
        )
        .bind(claimed.definition_id)
        .bind(reason)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;
        if updated.rows_affected() != 1 {
            return Err(OciWorkerStoreError::Conflict);
        }
        append_event(
            &mut transaction,
            claimed.definition_id,
            "state_changed",
            "failed",
        )
        .await?;
        transaction.commit().await.map_err(storage)
    }

    async fn claim_materialization(
        &self,
        worker_name: &str,
        lease: Duration,
    ) -> Result<Option<ClaimedMaterializationJob>, OciWorkerStoreError> {
        let lease_seconds = lease_seconds(lease)?;
        let row = sqlx::query_as::<_, MaterializationJobRow>(
            "WITH candidate AS (
                SELECT id
                  FROM oci_image_materialization_jobs
                 WHERE worker_name = $1
                   AND (state = 'queued'
                        OR (state = 'claimed' AND lease_expires_at <= now()))
                 ORDER BY created_at, id
                 FOR UPDATE SKIP LOCKED
                 LIMIT 1
             )
             UPDATE oci_image_materialization_jobs AS job
                SET state = 'claimed',
                    lease_expires_at = now() + make_interval(secs => $2),
                    updated_at = now()
               FROM candidate
              WHERE job.id = candidate.id
             RETURNING job.id, job.image_reference",
        )
        .bind(worker_name)
        .bind(lease_seconds)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?;
        row.map(MaterializationJobRow::try_into_job).transpose()
    }

    async fn complete_materialization(
        &self,
        job_id: Uuid,
        root_path: &Path,
    ) -> Result<(), OciWorkerStoreError> {
        if !root_path.is_absolute() {
            return Err(OciWorkerStoreError::Conflict);
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let image_reference: Option<String> = sqlx::query_scalar(
            "UPDATE oci_image_materialization_jobs
                SET state = 'materialized', lease_expires_at = NULL, root_path = $2,
                    failure_reason = NULL, updated_at = now()
              WHERE id = $1 AND state = 'claimed'
              RETURNING image_reference",
        )
        .bind(job_id)
        .bind(root_path.to_string_lossy().as_ref())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let _image_reference = image_reference.ok_or(OciWorkerStoreError::Conflict)?;
        transaction.commit().await.map_err(storage)
    }

    async fn fail_materialization(
        &self,
        job_id: Uuid,
        reason: &str,
    ) -> Result<(), OciWorkerStoreError> {
        validate_reason(reason)?;
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let image_reference: Option<String> = sqlx::query_scalar(
            "UPDATE oci_image_materialization_jobs
                SET state = 'failed', lease_expires_at = NULL, failure_reason = $2,
                    updated_at = now()
              WHERE id = $1 AND state = 'claimed'
              RETURNING image_reference",
        )
        .bind(job_id)
        .bind(reason)
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?;
        let _image_reference = image_reference.ok_or(OciWorkerStoreError::Conflict)?;
        transaction.commit().await.map_err(storage)
    }

    async fn materialized_roots(
        &self,
        worker_name: &str,
    ) -> Result<Vec<MaterializedRoot>, OciWorkerStoreError> {
        let rows = sqlx::query_as::<_, MaterializedRootRow>(
            "SELECT image_reference, root_path
               FROM oci_image_materialization_jobs
              WHERE worker_name = $1 AND state = 'materialized'
              ORDER BY image_reference",
        )
        .bind(worker_name)
        .fetch_all(&self.pool)
        .await
        .map_err(storage)?;
        rows.into_iter()
            .map(MaterializedRootRow::try_into_root)
            .collect()
    }
}
