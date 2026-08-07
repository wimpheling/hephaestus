//! `PostgreSQL` durable job adapter for isolated OCI image workers.

use async_trait::async_trait;
use builder_catalog_domain::{OciDigest, OciImageId, OciImageReference};
use oci_builder_worker::{
    ClaimedMaterializationJob, ClaimedProductionJob, MaterializedRoot, OciImageProductionJobStore,
    OciImageProductionOutput, OciWorkerError, OciWorkerStoreError, RepositoryOciImageProvenance,
    RepositoryOciImagePublicationLease, RepositoryOciImagePublicationStore,
    RepositoryOciImageSourcePath,
};
use registry_domain::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, PolicyVersion, PublicationIntent,
    PublicationIntentId, PublicationState, RegistryAuthority, RegistryNamespace, SupplyChainPolicy,
    VerifiedPublication,
};
use registry_postgres::PgRegistryStore;
use sqlx::{FromRow, PgPool, Postgres, Transaction};
use std::{path::Path, time::Duration};
use uuid::Uuid;

/// `PostgreSQL` implementation of the OCI production durable-job boundary.
#[derive(Clone)]
pub struct PgOciImageProductionJobStore {
    pool: PgPool,
}

/// Durable repository-image registry lifecycle adapter.
///
/// This adapter derives the Zot namespace solely from the durable project and
/// image identifiers, then delegates immutable lifecycle transitions to the
/// registry control plane. It never accepts a repository path from a build.
#[derive(Clone)]
pub struct PgRepositoryOciImagePublicationStore {
    pool: PgPool,
    registry: PgRegistryStore,
    authority: RegistryAuthority,
    policy_version: PolicyVersion,
    supply_chain_policy: SupplyChainPolicy,
}

impl PgRepositoryOciImagePublicationStore {
    /// Creates the adapter with fixed forge registry policy.
    #[must_use]
    pub const fn new(
        pool: PgPool,
        registry: PgRegistryStore,
        authority: RegistryAuthority,
        policy_version: PolicyVersion,
        supply_chain_policy: SupplyChainPolicy,
    ) -> Self {
        Self {
            pool,
            registry,
            authority,
            policy_version,
            supply_chain_policy,
        }
    }

    async fn assert_producing_owner(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
    ) -> Result<(), OciWorkerError> {
        let owned = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (
                SELECT 1 FROM repository_oci_image_definitions
                 WHERE id = $1 AND project_id = $2 AND status = 'producing'
             )",
        )
        .bind(image_id.as_uuid())
        .bind(project_id)
        .fetch_one(&self.pool)
        .await
        .map_err(|_| OciWorkerError::RegistryPublication)?;
        owned
            .then_some(())
            .ok_or(OciWorkerError::RegistryPublication)
    }

    fn intent(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
        expected_manifest: OciDescriptor,
    ) -> Result<PublicationIntent, OciWorkerError> {
        let claim = repository_image_claim(project_id, image_id)?;
        let reference = ImmutableManifestReference::new(
            self.authority.clone(),
            claim.namespace().clone(),
            expected_manifest.digest().clone(),
        );
        PublicationIntent::new(
            PublicationIntentId::new(),
            claim,
            reference,
            expected_manifest,
            self.policy_version.clone(),
            self.supply_chain_policy,
        )
        .map_err(|_| OciWorkerError::RegistryPublication)
    }
}

fn repository_image_claim(
    project_id: Uuid,
    image_id: OciImageId,
) -> Result<NamespaceClaim, OciWorkerError> {
    let namespace = RegistryNamespace::parse(format!(
        "projects/{project_id}/repository-images/{image_id}"
    ))
    .map_err(|_| OciWorkerError::RegistryPublication)?;
    Ok(NamespaceClaim::new(namespace.owner().clone()))
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

#[derive(Debug, FromRow)]
struct ProductionJobRow {
    id: Uuid,
    definition_id: Uuid,
}

#[derive(Debug, FromRow)]
struct ProductionInputRow {
    source_repository_id: Uuid,
    source_revision: String,
    project_id: Uuid,
    context_digest: String,
    dockerfile_path: String,
    context_path: String,
    base_image_reference: String,
}

#[async_trait]
impl RepositoryOciImagePublicationStore for PgRepositoryOciImagePublicationStore {
    async fn begin_repository_image_publication(
        &self,
        project_id: Uuid,
        image_id: OciImageId,
        expected_manifest: OciDescriptor,
    ) -> Result<RepositoryOciImagePublicationLease, OciWorkerError> {
        self.assert_producing_owner(project_id, image_id).await?;
        let requested = self.intent(project_id, image_id, expected_manifest)?;
        let stored = self
            .registry
            .create_intent(&requested)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)?;
        match stored.state() {
            PublicationState::Pending | PublicationState::Publishing => {
                let publishing = self
                    .registry
                    .begin_publishing(stored.id())
                    .await
                    .map_err(|_| OciWorkerError::RegistryPublication)?;
                Ok(RepositoryOciImagePublicationLease::Publish(publishing))
            }
            PublicationState::Verified | PublicationState::Approved => {
                Ok(RepositoryOciImagePublicationLease::Approved(stored))
            }
            PublicationState::Retired | PublicationState::Missing => {
                Err(OciWorkerError::RegistryPublication)
            }
        }
    }

    async fn record_verified_and_approve(
        &self,
        intent_id: PublicationIntentId,
        verification: VerifiedPublication,
    ) -> Result<PublicationIntent, OciWorkerError> {
        let verified = self
            .registry
            .record_verified(intent_id, verification)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)?;
        if !matches!(
            verified.state(),
            PublicationState::Verified | PublicationState::Approved
        ) {
            return Err(OciWorkerError::RegistryPublication);
        }
        self.registry
            .approve(intent_id)
            .await
            .map_err(|_| OciWorkerError::RegistryPublication)
    }

    async fn retry_repository_image_publication(
        &self,
        intent_id: PublicationIntentId,
    ) -> Result<(), OciWorkerError> {
        self.registry
            .retry(intent_id)
            .await
            .map(|_| ())
            .map_err(|_| OciWorkerError::RegistryPublication)
    }
}

#[derive(Debug, FromRow)]
struct MaterializationJobRow {
    id: Uuid,
    image_reference: String,
}

impl MaterializationJobRow {
    fn try_into_job(self) -> Result<ClaimedMaterializationJob, OciWorkerStoreError> {
        Ok(ClaimedMaterializationJob {
            id: self.id,
            image_reference: parse_reference(self.image_reference)?,
        })
    }
}

#[derive(Debug, FromRow)]
struct MaterializedRootRow {
    image_reference: String,
    root_path: String,
}

impl MaterializedRootRow {
    fn try_into_root(self) -> Result<MaterializedRoot, OciWorkerStoreError> {
        let root_path = std::path::PathBuf::from(self.root_path);
        if !root_path.is_absolute() {
            return Err(OciWorkerStoreError::Conflict);
        }
        Ok(MaterializedRoot {
            image_reference: parse_reference(self.image_reference)?,
            root_path,
        })
    }
}

async fn append_event(
    transaction: &mut Transaction<'_, Postgres>,
    image_id: Uuid,
    change_kind: &str,
    status: &str,
) -> Result<(), OciWorkerStoreError> {
    let event_id = sqlx::query_scalar::<_, Uuid>(
        "SELECT event.event_id
         FROM repository_oci_image_definitions AS definition
         CROSS JOIN LATERAL append_application_event(
            gen_random_uuid(), 'project', definition.project_id, 'project', definition.project_id,
            'project.changed', $2, $3, $1, NULL
         ) AS event
         WHERE definition.id = $1",
    )
    .bind(image_id)
    .bind(change_kind)
    .bind(status)
    .fetch_optional(&mut **transaction)
    .await
    .map_err(storage)?;
    event_id.map(|_| ()).ok_or(OciWorkerStoreError::Conflict)
}

fn validate_output(
    output: &OciImageProductionOutput,
    provenance: &RepositoryOciImageProvenance,
) -> Result<(), OciWorkerStoreError> {
    if output.image_reference.digest().map_err(invalid)? != output.image_digest
        || output.scan_reference.trim().is_empty()
        || output.scan_reference.len() > 2048
    {
        return Err(OciWorkerStoreError::Conflict);
    }
    provenance.validate().map_err(invalid)
}

fn parse_reference(value: String) -> Result<OciImageReference, OciWorkerStoreError> {
    OciImageReference::parse(value).map_err(invalid)
}

fn parse_digest(value: String) -> Result<OciDigest, OciWorkerStoreError> {
    OciDigest::parse(value).map_err(invalid)
}

fn parse_path(value: String) -> Result<RepositoryOciImageSourcePath, OciWorkerStoreError> {
    RepositoryOciImageSourcePath::parse(value).map_err(invalid)
}

fn lease_seconds(lease: Duration) -> Result<i64, OciWorkerStoreError> {
    i64::try_from(lease.as_secs())
        .ok()
        .filter(|seconds| *seconds > 0)
        .ok_or(OciWorkerStoreError::Conflict)
}

fn validate_reason(reason: &str) -> Result<(), OciWorkerStoreError> {
    (!reason.trim().is_empty() && reason.len() <= 2048)
        .then_some(())
        .ok_or(OciWorkerStoreError::Conflict)
}

fn invalid(error: impl std::error::Error + Send + Sync + 'static) -> OciWorkerStoreError {
    OciWorkerStoreError::Storage(Box::new(error))
}

fn storage(error: impl std::error::Error + Send + Sync + 'static) -> OciWorkerStoreError {
    OciWorkerStoreError::Storage(Box::new(error))
}

#[cfg(test)]
mod tests {
    use super::repository_image_claim;
    use builder_catalog_domain::OciImageId;
    use registry_domain::{
        ImmutableManifestReference, OciDescriptor, OciMediaType, PolicyVersion, PublicationIntent,
        PublicationIntentId, RegistryAuthority, Sha256Digest, SupplyChainPolicy,
    };
    use uuid::Uuid;

    #[test]
    fn namespace_uses_only_opaque_project_and_image_ids() {
        let project_id = Uuid::from_u128(1);
        let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
        let claim = repository_image_claim(project_id, image_id).expect("namespace claim");
        assert_eq!(
            claim.namespace().as_str(),
            "projects/00000000-0000-0000-0000-000000000001/repository-images/00000000-0000-0000-0000-000000000002"
        );
    }

    #[test]
    fn project_changes_produce_a_different_namespace_for_the_same_image() {
        let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
        let first =
            repository_image_claim(Uuid::from_u128(1), image_id).expect("first namespace claim");
        let second =
            repository_image_claim(Uuid::from_u128(3), image_id).expect("second namespace claim");
        assert_ne!(first.namespace(), second.namespace());
    }

    #[test]
    fn cross_project_intent_mismatch_is_rejected_before_it_reaches_zot() {
        let image_id = OciImageId::from_uuid(Uuid::from_u128(2));
        let first =
            repository_image_claim(Uuid::from_u128(1), image_id).expect("first namespace claim");
        let second =
            repository_image_claim(Uuid::from_u128(3), image_id).expect("second namespace claim");
        let descriptor = OciDescriptor::new(
            Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
            42,
            OciMediaType::parse(OciMediaType::IMAGE_INDEX).expect("media type"),
        )
        .expect("descriptor");
        let reference = ImmutableManifestReference::new(
            RegistryAuthority::parse("registry.example").expect("authority"),
            second.namespace().clone(),
            descriptor.digest().clone(),
        );
        assert!(
            PublicationIntent::new(
                PublicationIntentId::from_uuid(Uuid::from_u128(4)),
                first,
                reference,
                descriptor,
                PolicyVersion::parse("image-v1").expect("policy version"),
                SupplyChainPolicy::without_signature(),
            )
            .is_err()
        );
    }

    #[test]
    fn interrupted_publication_intents_return_to_the_retryable_pending_state() {
        let claim = repository_image_claim(
            Uuid::from_u128(1),
            OciImageId::from_uuid(Uuid::from_u128(2)),
        )
        .expect("namespace claim");
        let descriptor = OciDescriptor::new(
            Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).expect("digest"),
            42,
            OciMediaType::parse(OciMediaType::IMAGE_INDEX).expect("media type"),
        )
        .expect("descriptor");
        let reference = ImmutableManifestReference::new(
            RegistryAuthority::parse("registry.example").expect("authority"),
            claim.namespace().clone(),
            descriptor.digest().clone(),
        );
        let intent = PublicationIntent::new(
            PublicationIntentId::from_uuid(Uuid::from_u128(4)),
            claim,
            reference,
            descriptor,
            PolicyVersion::parse("image-v1").expect("policy version"),
            SupplyChainPolicy::without_signature(),
        )
        .expect("intent")
        .begin_publishing()
        .expect("publishing")
        .retry()
        .expect("retry");
        assert!(matches!(
            intent.state(),
            registry_domain::PublicationState::Pending
        ));
    }
}
