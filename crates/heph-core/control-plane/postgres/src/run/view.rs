//! Authorized run detail loading and event decoding.

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sqlx::{FromRow, types::Json};
use time::OffsetDateTime;
use uuid::Uuid;

use super::artifact::{PreviewArtifact, ResultPreviews, load_previews};
use super::model::{EventPayload, RunApplication, RunArtifact, RunError, RunEvent, RunView};

impl RunApplication {
    pub async fn get_run(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<RunView, RunError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        let run = sqlx::query_as::<_, RunViewRow>(
            "SELECT run.id, run.state, run.outcome, run.exit_code, run.exit_signal, run.failure,
                    run.created_at, run.updated_at, run.state_version, instance.id AS agent_id,
                    instance.name AS agent_name, instance_project.id AS instance_project_id,
                    instance_project.name AS instance_project_name, run.instance_revision_id,
                    run.release_id, release.version AS release_version,
                    release.repository_id AS source_repository_id, repository.id AS repository_id,
                    repository.name AS repository_name, project.id AS project_id,
                    project.name AS project_name, organization.id AS organization_id,
                    organization.name AS organization_name,
                    COALESCE(request.commit_sha, delivery.target_commit) AS input_commit,
                    COALESCE(request.git_ref, delivery.target_ref) AS git_ref,
                    COALESCE(request.attempt, delivery.attempt_number) AS attempt,
                    EXISTS (SELECT 1 FROM run_requests retry_request
                            WHERE retry_request.run_id = run.id) AS retry_supported,
                    result.id AS result_id, result.result_commit,
                    result.result_ref, result.result_tree, result.message AS result_message,
                    result.artifact_manifest_hash, proposal.id AS proposal_id,
                    proposal.state AS proposal_state, proposal.target_ref AS proposal_target_ref,
                    proposal.version AS proposal_version
             FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
             JOIN projects instance_project ON instance_project.id = instance.project_id
             JOIN releases release ON release.id = run.release_id
             LEFT JOIN run_requests request ON request.run_id = run.id
             LEFT JOIN mailbox_delivery_attempts delivery ON delivery.run_id = run.id
             LEFT JOIN agent_attachments attachment ON attachment.id = run.attachment_id
             JOIN repositories repository
               ON repository.id = COALESCE(request.repository_id, attachment.repository_id)
             JOIN projects project ON project.id = repository.project_id
             JOIN organizations organization ON organization.id = project.organization_id
             LEFT JOIN run_results result ON result.run_id = run.id
             LEFT JOIN review_proposals proposal ON proposal.run_id = run.id
             WHERE run.id = $1 AND check_permission('user', hephaestus_actor_id(),
                 'can_read', 'run', run.id::text) = 1",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(RunError::Persistence)?
        .ok_or(RunError::NotFound)?;
        let events = sqlx::query_as::<_, EventRow>("SELECT sequence, event_type, payload, occurred_at FROM run_events WHERE run_id = $1 ORDER BY sequence")
            .bind(id).fetch_all(&mut *tx).await.map_err(RunError::Persistence)?.into_iter().map(parse_event).collect();
        let artifacts = if let Some(result_id) = run.result_id {
            sqlx::query_as::<_, RunArtifact>("SELECT id, kind, path, git_mode AS mode, media_type, size_bytes, sha256, storage_key FROM result_artifacts WHERE result_id = $1 ORDER BY kind, path, id")
                .bind(result_id).fetch_all(&mut *tx).await.map_err(RunError::Persistence)?
        } else {
            Vec::new()
        };
        tx.commit().await.map_err(RunError::Persistence)?;
        let artifact_root = self.result_artifact_root.clone();
        let preview_artifacts = artifacts
            .iter()
            .filter(|artifact| matches!(artifact.kind.as_str(), "patch" | "manifest"))
            .map(PreviewArtifact::from)
            .collect::<Vec<_>>();
        let previews = tokio::task::spawn_blocking(move || {
            load_previews(&artifact_root, id, &preview_artifacts)
        })
        .await
        .map_err(|_| RunError::PreviewUnavailable)??;
        Ok(run.into_view(events, artifacts, previews))
    }
}
#[derive(FromRow)]
struct RunViewRow {
    id: Uuid,
    state: String,
    outcome: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
    failure: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    state_version: i64,
    agent_id: Uuid,
    agent_name: String,
    instance_project_id: Uuid,
    instance_project_name: String,
    instance_revision_id: Uuid,
    release_id: Uuid,
    release_version: String,
    source_repository_id: Uuid,
    repository_id: Uuid,
    repository_name: String,
    project_id: Uuid,
    project_name: String,
    organization_id: Uuid,
    organization_name: String,
    input_commit: String,
    git_ref: String,
    attempt: i32,
    retry_supported: bool,
    result_id: Option<Uuid>,
    result_commit: Option<String>,
    result_ref: Option<String>,
    result_tree: Option<String>,
    result_message: Option<String>,
    artifact_manifest_hash: Option<String>,
    proposal_id: Option<Uuid>,
    proposal_state: Option<String>,
    proposal_target_ref: Option<String>,
    proposal_version: Option<i64>,
}

impl RunViewRow {
    fn into_view(
        self,
        events: Vec<RunEvent>,
        artifacts: Vec<RunArtifact>,
        previews: ResultPreviews,
    ) -> RunView {
        RunView {
            id: self.id,
            state: self.state,
            outcome: self.outcome,
            exit_code: self.exit_code,
            exit_signal: self.exit_signal,
            failure: self.failure,
            created_at: self.created_at,
            updated_at: self.updated_at,
            state_version: self.state_version,
            agent_id: self.agent_id,
            agent_name: self.agent_name,
            instance_project_id: self.instance_project_id,
            instance_project_name: self.instance_project_name,
            instance_revision_id: self.instance_revision_id,
            release_id: self.release_id,
            release_version: self.release_version,
            source_repository_id: self.source_repository_id,
            repository_id: self.repository_id,
            repository_name: self.repository_name,
            project_id: self.project_id,
            project_name: self.project_name,
            organization_id: self.organization_id,
            organization_name: self.organization_name,
            input_commit: self.input_commit,
            git_ref: self.git_ref,
            attempt: self.attempt,
            retry_supported: self.retry_supported,
            result_id: self.result_id,
            result_commit: self.result_commit,
            result_ref: self.result_ref,
            result_tree: self.result_tree,
            result_message: self.result_message,
            artifact_manifest_hash: self.artifact_manifest_hash,
            proposal_id: self.proposal_id,
            proposal_state: self.proposal_state,
            proposal_target_ref: self.proposal_target_ref,
            proposal_version: self.proposal_version,
            events,
            artifacts,
            patch_preview: previews.patch,
            manifest_preview: previews.manifest,
        }
    }
}

#[derive(FromRow)]
struct EventRow {
    sequence: i64,
    event_type: String,
    payload: Json<Value>,
    occurred_at: OffsetDateTime,
}
fn parse_event(row: EventRow) -> RunEvent {
    let payload = if row.event_type == "vm.log" {
        let bytes = row
            .payload
            .get("bytes")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_u64)
            .filter_map(|n| u8::try_from(n).ok())
            .take(4096)
            .collect::<Vec<_>>();
        EventPayload::Log(String::from_utf8_lossy(&bytes).into_owned())
    } else if row.event_type == "vm.metric" {
        let labels = row
            .payload
            .get("labels")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_str().map(|v| (k.clone(), v.to_owned())))
            .collect();
        EventPayload::Metric {
            name: row
                .payload
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            value: row
                .payload
                .get("value")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
            labels,
        }
    } else {
        EventPayload::State(row.event_type.clone())
    };
    RunEvent {
        sequence: row.sequence,
        event_type: row.event_type,
        payload,
        occurred_at: row.occurred_at,
    }
}
