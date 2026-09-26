use async_trait::async_trait;
use run_domain::{Run, RunKind};
use run_orchestrator::{RunRuntimeCatalog, RunRuntimeCatalogError, RunRuntimeInput};
use runtime_types::RunId;

use crate::PgRunRepository;

use super::{
    models::RuntimeContextRow,
    utility::{run_kind_name, storage},
};

#[async_trait]
impl RunRuntimeCatalog for PgRunRepository {
    async fn load_runtime(&self, run: &Run) -> Result<RunRuntimeInput, RunRuntimeCatalogError> {
        let context = sqlx::query_as::<_, RuntimeContextRow>(
            "SELECT revision.parameters,
                    CASE WHEN revision.publication_mode = 'runtime_git'
                         THEN provenance.target_repository_id
                         ELSE COALESCE(request.repository_id, attachment.repository_id)
                    END AS repository_id,
                    CASE WHEN revision.publication_mode = 'runtime_git'
                         THEN provenance.target_ref
                         ELSE COALESCE(request.git_ref, mailbox_attempt.target_ref)
                    END AS git_ref,
                    CASE WHEN revision.publication_mode = 'runtime_git'
                         THEN provenance.target_commit
                         ELSE COALESCE(request.commit_sha, mailbox_attempt.target_commit)
                    END AS commit_sha,
                    release.state AS release_state,
                    update.id AS update_id,
                    update.expected_current_revision_id AS previous_revision_id,
                    previous_agent.release_id AS previous_release_id,
                    previous.parameters AS previous_parameters
             FROM runs AS stored_run
             JOIN agent_instance_revisions AS revision
               ON revision.id = stored_run.instance_revision_id
              AND revision.instance_id = stored_run.instance_id
             JOIN release_agents AS release_agent
               ON release_agent.id = stored_run.release_agent_id
              AND release_agent.release_id = stored_run.release_id
              AND revision.release_agent_id = release_agent.id
             JOIN releases AS release ON release.id = stored_run.release_id
             LEFT JOIN run_requests AS request ON request.run_id = stored_run.id
             LEFT JOIN mailbox_delivery_attempts AS mailbox_attempt ON mailbox_attempt.run_id = stored_run.id
             LEFT JOIN agent_attachments AS attachment
               ON attachment.id = stored_run.attachment_id
              AND attachment.instance_id = stored_run.instance_id
             LEFT JOIN run_instance_provenance AS provenance
               ON provenance.run_id = stored_run.id
              AND provenance.instance_id = stored_run.instance_id
              AND provenance.instance_revision_id = stored_run.instance_revision_id
             LEFT JOIN agent_updates AS update
               ON update.hook_run_id = stored_run.id
             LEFT JOIN agent_instance_revisions AS previous
               ON previous.id = update.expected_current_revision_id
              AND previous.instance_id = stored_run.instance_id
             LEFT JOIN release_agents AS previous_agent
               ON previous_agent.id = previous.release_agent_id
             WHERE stored_run.id = $1
               AND stored_run.instance_id = $2
               AND stored_run.instance_revision_id = $3
               AND stored_run.release_id = $4
               AND stored_run.release_agent_id = $5
               AND stored_run.run_kind = $6",
        )
        .bind(run.id.as_uuid())
        .bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid())
        .bind(run.release_id.as_uuid())
        .bind(run.release_agent_id.as_uuid())
        .bind(run_kind_name(run.kind))
        .fetch_optional(&self.pool)
        .await
        .map_err(storage)?
        .ok_or(RunRuntimeCatalogError::Unavailable)?;

        if context.release_state != "published" {
            return Err(RunRuntimeCatalogError::InvalidData(
                "release is not published",
            ));
        }
        if run.kind == RunKind::Normal
            && (context.repository_id.is_none()
                || context.git_ref.is_none()
                || context.commit_sha.is_none())
        {
            return Err(RunRuntimeCatalogError::InvalidData(
                "normal run target provenance",
            ));
        }

        let artifacts = self
            .load_runtime_artifacts(run.release_id.as_uuid())
            .await?;
        let previous_artifacts = match context.previous_release_id {
            Some(release_id) => self.load_runtime_artifacts(release_id).await?,
            None => Vec::new(),
        };
        let mailbox_event = self.load_mailbox_event(run.id).await?;
        if self.is_retry_run(run.id).await? {
            self.validate_retry_lineage(run.id).await?;
        }

        Ok(RunRuntimeInput {
            parameters: context.parameters,
            repository_id: context.repository_id,
            git_ref: context.git_ref,
            commit_sha: context.commit_sha,
            update_id: context.update_id,
            previous_revision_id: context.previous_revision_id,
            previous_release_id: context.previous_release_id,
            previous_parameters: context.previous_parameters,
            artifacts,
            previous_artifacts,
            mailbox_event,
        })
    }

    async fn run_is_live(&self, run_id: RunId) -> Result<bool, RunRuntimeCatalogError> {
        sqlx::query_scalar(
            "SELECT EXISTS(
                SELECT 1 FROM runs
                WHERE id = $1 AND state <> 'cleaned_up'
             )",
        )
        .bind(run_id.as_uuid())
        .fetch_one(&self.pool)
        .await
        .map_err(storage)
    }
}
