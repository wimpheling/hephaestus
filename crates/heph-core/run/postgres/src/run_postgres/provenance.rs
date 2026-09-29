use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::{Run, RunKind};
use run_orchestrator::RepositoryError;
use runtime_types::AgentAttachmentId;
use uuid::Uuid;

use super::{PgRunRepository, errors::storage};

#[derive(Debug, sqlx::FromRow)]
struct RuntimeGitProvenance {
    publication_mode: String,
    trigger_repository_id: Option<Uuid>,
    git_ref: Option<String>,
    commit_sha: Option<String>,
    attachment_id: Option<Uuid>,
    git_binding_id: Option<Uuid>,
    target_repository_id: Option<Uuid>,
    parameter_hash: Vec<u8>,
    platform_policy_version: String,
}

#[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
struct ProvenanceRow {
    instance_id: Uuid,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    attachment_id: Option<Uuid>,
    target_repository_id: Option<Uuid>,
    target_ref: Option<String>,
    target_commit: Option<String>,
    parameter_hash: Vec<u8>,
    platform_policy_version: String,
    phase: String,
    authorization_model_version: String,
}
// Keep the derivation, insert, and immutable replay comparison together so
// the security-sensitive target capture remains auditable in one boundary.
#[allow(clippy::too_many_lines)]
impl PgRunRepository {
    pub(super) async fn ensure_runtime_git_provenance_impl(
        &self,
        run: &Run,
    ) -> Result<(), RepositoryError> {
        if run.kind != RunKind::Normal {
            return Ok(());
        }
        let mut transaction = self.pool.begin().await.map_err(storage)?;
        let expected: RuntimeGitProvenance = sqlx::query_as(
            "SELECT revision.publication_mode,
                    request.repository_id AS trigger_repository_id,
                    request.git_ref,
                    request.commit_sha,
                    request.attachment_id,
                    git.binding_id AS git_binding_id,
                    generic.resource_id AS target_repository_id,
                    revision.parameter_hash,
                    revision.platform_policy_version
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             LEFT JOIN run_requests AS request
               ON request.run_id = stored_run.id
              AND request.instance_id = stored_run.instance_id
              AND request.instance_revision_id = stored_run.instance_revision_id
              AND request.release_id = stored_run.release_id
              AND request.release_agent_id = stored_run.release_agent_id
             LEFT JOIN agent_capability_bindings AS generic
               ON generic.id = revision.publication_repository_binding_id
              AND generic.instance_revision_id = revision.id
              AND generic.resource_kind = 'repository'
             LEFT JOIN agent_git_capability_bindings AS git
               ON git.binding_id = generic.id
              AND git.instance_revision_id = revision.id
             WHERE stored_run.id = $1
               AND stored_run.instance_id = $2
               AND stored_run.instance_revision_id = $3
               AND stored_run.release_id = $4
               AND stored_run.release_agent_id = $5
             FOR UPDATE OF stored_run",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(RepositoryError::InvalidData("runtime Git run"))?;

        if expected.publication_mode != "runtime_git" {
            transaction.commit().await.map_err(storage)?;
            return Ok(());
        }
        let attachment_id = run.attachment_id.map(AgentAttachmentId::as_uuid).ok_or(
            RepositoryError::InvalidData("runtime Git normal run attachment"),
        )?;
        let target_repository_id =
            expected
                .target_repository_id
                .ok_or(RepositoryError::InvalidData(
                    "runtime Git publication repository capability",
                ))?;
        if expected.git_binding_id.is_none()
            || expected.trigger_repository_id.is_none()
            || expected.attachment_id != Some(attachment_id)
            || expected.git_ref.is_none()
            || expected.commit_sha.is_none()
        {
            return Err(RepositoryError::InvalidData(
                "runtime Git immutable run request",
            ));
        }

        sqlx::query(
            "INSERT INTO run_instance_provenance
               (run_id, instance_id, instance_revision_id, release_id,
                release_agent_id, attachment_id, target_repository_id,
                target_ref, target_commit, parameter_hash,
                platform_policy_version, phase, authorization_model_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10,
                     $11, 'normal', $12)
             ON CONFLICT (run_id) DO NOTHING",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(attachment_id)
        .bind(target_repository_id)
        .bind(expected.git_ref.as_deref())
        .bind(expected.commit_sha.as_deref())
        .bind(&expected.parameter_hash)
        .bind(&expected.platform_policy_version)
        .bind(AUTHORIZATION_MODEL_VERSION)
        .execute(&mut *transaction)
        .await
        .map_err(storage)?;

        let actual: ProvenanceRow = sqlx::query_as(
            "SELECT instance_id, instance_revision_id, release_id,
                    release_agent_id, attachment_id, target_repository_id,
                    target_ref, target_commit, parameter_hash,
                    platform_policy_version, phase, authorization_model_version
             FROM run_instance_provenance
             WHERE run_id = $1
             FOR UPDATE",
        )
        .bind(run.id.as_uuid())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(storage)?
        .ok_or(RepositoryError::InvalidData(
            "runtime Git provenance disappeared",
        ))?;
        let expected_provenance = ProvenanceRow {
            instance_id: run.instance_id.as_uuid(),
            instance_revision_id: run.instance_revision_id.as_uuid(),
            release_id: run.release_id.as_uuid(),
            release_agent_id: run.release_agent_id.as_uuid(),
            attachment_id: Some(attachment_id),
            target_repository_id: Some(target_repository_id),
            target_ref: Some(
                expected
                    .git_ref
                    .as_deref()
                    .ok_or(RepositoryError::InvalidData("runtime Git ref"))?
                    .to_owned(),
            ),
            target_commit: Some(
                expected
                    .commit_sha
                    .as_deref()
                    .ok_or(RepositoryError::InvalidData("runtime Git commit"))?
                    .to_owned(),
            ),
            parameter_hash: expected.parameter_hash.clone(),
            platform_policy_version: expected.platform_policy_version.clone(),
            phase: String::from("normal"),
            authorization_model_version: String::from(AUTHORIZATION_MODEL_VERSION),
        };
        if actual != expected_provenance {
            return Err(RepositoryError::InvalidData(
                "runtime Git provenance conflict",
            ));
        }
        transaction.commit().await.map_err(storage)?;
        Ok(())
    }
}
