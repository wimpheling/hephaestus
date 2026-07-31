//! Authorized run reads and durable control request creation.

use authz_postgres::begin_actor_transaction;
use identity_domain::AuthenticatedIdentity;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool, types::Json};
use std::{
    collections::BTreeMap,
    fs::OpenOptions,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use time::OffsetDateTime;
use uuid::Uuid;

const MAX_RESULT_PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("run was not found")]
    NotFound,
    #[error("run persistence failed")]
    Persistence(#[source] sqlx::Error),
    #[error("run page is invalid")]
    InvalidPage,
    #[error("control request conflicts with an earlier retry")]
    IdempotencyConflict,
    #[error("run result preview is unavailable")]
    PreviewUnavailable,
}

#[derive(Clone, Copy)]
pub struct Page {
    pub size: i64,
    pub after: Option<Uuid>,
}
pub struct PageResult<T> {
    pub values: Vec<T>,
    pub next: Option<String>,
}

#[derive(FromRow)]
pub struct RunSummary {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub run_kind: String,
    pub updated_at: OffsetDateTime,
    pub instance_id: Uuid,
    pub instance_name: String,
    pub repository_id: Option<Uuid>,
    pub repository_name: Option<String>,
    pub commit_sha: Option<String>,
    pub git_ref: Option<String>,
    pub release_id: Uuid,
    pub release_version: String,
    pub instance_revision_id: Uuid,
}

#[derive(FromRow)]
pub struct RunView {
    pub id: Uuid,
    pub state: String,
    pub outcome: Option<String>,
    pub exit_code: Option<i32>,
    pub exit_signal: Option<i32>,
    pub failure: Option<String>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub state_version: i64,
    pub agent_id: Uuid,
    pub agent_name: String,
    pub instance_project_id: Uuid,
    pub instance_project_name: String,
    pub instance_revision_id: Uuid,
    pub release_id: Uuid,
    pub release_version: String,
    pub source_repository_id: Uuid,
    pub repository_id: Uuid,
    pub repository_name: String,
    pub project_id: Uuid,
    pub project_name: String,
    pub organization_id: Uuid,
    pub organization_name: String,
    pub input_commit: String,
    pub git_ref: String,
    pub attempt: i32,
    pub result_id: Option<Uuid>,
    pub result_commit: Option<String>,
    pub result_ref: Option<String>,
    pub result_tree: Option<String>,
    pub result_message: Option<String>,
    pub artifact_manifest_hash: Option<String>,
    pub proposal_id: Option<Uuid>,
    pub proposal_state: Option<String>,
    pub proposal_target_ref: Option<String>,
    pub proposal_version: Option<i64>,
    pub events: Vec<RunEvent>,
    pub artifacts: Vec<RunArtifact>,
    pub patch_preview: Option<String>,
    pub manifest_preview: Option<String>,
}

pub struct RunEvent {
    pub sequence: i64,
    pub event_type: String,
    pub payload: EventPayload,
    pub occurred_at: OffsetDateTime,
}
pub enum EventPayload {
    Log(String),
    Metric {
        name: String,
        value: f64,
        labels: BTreeMap<String, String>,
    },
    State(String),
}

#[derive(FromRow)]
pub struct RunArtifact {
    pub id: Uuid,
    pub kind: String,
    pub path: String,
    pub mode: Option<i32>,
    pub media_type: Option<String>,
    pub size_bytes: i64,
    pub sha256: String,
    pub(crate) storage_key: String,
}

pub enum ControlKind {
    Cancel,
    Retry,
    Approve,
    Reject,
}
pub enum ControlTarget {
    Run(Uuid),
    Proposal(Uuid),
}
pub struct RequestControl {
    pub kind: ControlKind,
    pub repository_id: Uuid,
    pub target: ControlTarget,
    pub reason: String,
}
pub struct RequestedControl {
    pub id: Uuid,
    pub state: String,
}

pub struct RunApplication {
    pool: PgPool,
    result_artifact_root: PathBuf,
}

/// Immutable launch contract loaded for a run before guest construction.
#[derive(FromRow)]
pub struct VmLaunchContract {
    pub runtime_contract: Value,
    pub effective_runtime_policy: Value,
    pub requires_state: bool,
    pub update_hook: Option<Value>,
    pub release_state: String,
    pub revision_runnable: bool,
    pub attachment_runnable: bool,
    pub agent_update_id: Option<Uuid>,
}

/// Loads the immutable release, revision, attachment, and update-hook contract.
pub async fn load_vm_launch_contract(
    pool: &PgPool,
    run_id: Uuid,
) -> Result<Option<VmLaunchContract>, sqlx::Error> {
    sqlx::query_as(
        "SELECT release_agent.runtime_contract,
                revision.effective_runtime_policy,
                release_agent.requires_state,
                release_agent.update_hook,
                release.state,
                revision.runnable AS revision_runnable,
                COALESCE((
                    (run.run_kind = 'update' AND instance.state = 'updating')
                    OR (
                        run.run_kind = 'normal'
                        AND instance.state IN ('active', 'update_rejected')
                        AND instance.active_revision_id = revision.id
                        AND attachment.enabled
                        AND attachment.removed_at IS NULL
                    )
                ), false) AS attachment_runnable,
                agent_update.id AS agent_update_id
         FROM runs AS run
         JOIN agent_instances AS instance ON instance.id = run.instance_id
         JOIN agent_instance_revisions AS revision
           ON revision.id = run.instance_revision_id
          AND revision.instance_id = run.instance_id
         JOIN release_agents AS release_agent
           ON release_agent.id = run.release_agent_id
          AND release_agent.release_id = run.release_id
          AND revision.release_agent_id = release_agent.id
         JOIN releases AS release ON release.id = run.release_id
         LEFT JOIN agent_attachments AS attachment
           ON attachment.id = run.attachment_id
          AND attachment.instance_id = run.instance_id
         LEFT JOIN agent_updates AS agent_update
           ON agent_update.hook_run_id = run.id
         WHERE run.id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
}

/// Lists update hook runs whose completion needs reconciliation after restart.
pub async fn recoverable_update_hook_run_ids(pool: &PgPool) -> Result<Vec<Uuid>, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT update.hook_run_id
         FROM agent_updates AS update
         JOIN runs AS run ON run.id = update.hook_run_id
         WHERE update.state IN ('hook_running', 'hook_committed')
           AND run.state = 'cleaned_up'
         ORDER BY update.created_at, update.id",
    )
    .fetch_all(pool)
    .await
}

impl RunApplication {
    pub const fn new(pool: PgPool, result_artifact_root: PathBuf) -> Self {
        Self {
            pool,
            result_artifact_root,
        }
    }

    pub async fn list_project_runs(
        &self,
        identity: &AuthenticatedIdentity,
        project_id: Uuid,
        page: Page,
    ) -> Result<PageResult<RunSummary>, RunError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        let mut values = sqlx::query_as::<_, RunSummary>(
            "SELECT run.id, run.state, run.outcome, run.run_kind, run.updated_at,
                    instance.id AS instance_id, instance.name AS instance_name,
                    request.repository_id, repository.name AS repository_name,
                    request.commit_sha, request.git_ref, release.id AS release_id,
                    release.version AS release_version, run.instance_revision_id
             FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
             LEFT JOIN run_requests request ON request.run_id = run.id
             LEFT JOIN repositories repository ON repository.id = request.repository_id
             JOIN releases release ON release.id = run.release_id
             WHERE instance.project_id = $1 AND ($2::uuid IS NULL OR (run.created_at, run.id) <
                 (SELECT cursor.created_at, cursor.id FROM runs cursor WHERE cursor.id = $2))
             ORDER BY run.created_at DESC, run.id DESC LIMIT $3",
        )
        .bind(project_id)
        .bind(page.after)
        .bind(page.size + 1)
        .fetch_all(&mut *tx)
        .await
        .map_err(RunError::Persistence)?;
        tx.commit().await.map_err(RunError::Persistence)?;
        let size = usize::try_from(page.size).map_err(|_| RunError::InvalidPage)?;
        let has_more = values.len() > size;
        values.truncate(size);
        let next = has_more
            .then(|| values.last())
            .flatten()
            .map(|row| row.id.to_string());
        Ok(PageResult { values, next })
    }

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
                    organization.name AS organization_name, request.commit_sha AS input_commit,
                    request.git_ref, request.attempt, result.id AS result_id, result.result_commit,
                    result.result_ref, result.result_tree, result.message AS result_message,
                    result.artifact_manifest_hash, proposal.id AS proposal_id,
                    proposal.state AS proposal_state, proposal.target_ref AS proposal_target_ref,
                    proposal.version AS proposal_version
             FROM runs run JOIN agent_instances instance ON instance.id = run.instance_id
             JOIN projects instance_project ON instance_project.id = instance.project_id
             JOIN releases release ON release.id = run.release_id
             JOIN run_requests request ON request.run_id = run.id
             JOIN repositories repository ON repository.id = request.repository_id
             JOIN projects project ON project.id = repository.project_id
             JOIN organizations organization ON organization.id = project.organization_id
             LEFT JOIN run_results result ON result.run_id = run.id
             LEFT JOIN review_proposals proposal ON proposal.run_id = run.id WHERE run.id = $1",
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

    pub async fn request_control(
        &self,
        identity: &AuthenticatedIdentity,
        request: RequestControl,
    ) -> Result<RequestedControl, RunError> {
        let mut tx = begin_actor_transaction(&self.pool, identity)
            .await
            .map_err(RunError::Persistence)?;
        let kind = match request.kind {
            ControlKind::Cancel => "cancel_run",
            ControlKind::Retry => "retry_run",
            ControlKind::Approve => "approve_result",
            ControlKind::Reject => "reject_result",
        };
        let (run_id, proposal_id) = match request.target {
            ControlTarget::Run(id) => (Some(id), None),
            ControlTarget::Proposal(id) => (None, Some(id)),
        };
        if let Some(row) = sqlx::query_as::<_, ControlRow>("SELECT id, kind, repository_id, run_id, proposal_id, reason, state FROM control_requests WHERE actor_id = $1 AND request_id = $2")
            .bind(identity.user_id.as_uuid()).bind(identity.idempotency_id.as_uuid()).fetch_optional(&mut *tx).await.map_err(RunError::Persistence)? {
            if row.kind != kind || row.repository_id != request.repository_id || row.run_id != run_id || row.proposal_id != proposal_id || row.reason != request.reason { return Err(RunError::IdempotencyConflict); }
            tx.commit().await.map_err(RunError::Persistence)?;
            return Ok(RequestedControl { id: row.id, state: row.state });
        }
        let id = stable_id(identity, kind);
        let state = sqlx::query_scalar("INSERT INTO control_requests (id, kind, actor_id, request_id, repository_id, run_id, proposal_id, reason) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING state")
            .bind(id).bind(kind).bind(identity.user_id.as_uuid()).bind(identity.idempotency_id.as_uuid()).bind(request.repository_id).bind(run_id).bind(proposal_id).bind(request.reason)
            .fetch_one(&mut *tx).await.map_err(RunError::Persistence)?;
        tx.commit().await.map_err(RunError::Persistence)?;
        Ok(RequestedControl { id, state })
    }
}

#[cfg(test)]
mod reconciliation_tests {
    use crate::revoked_raw_run_ids;
    use sqlx::postgres::PgPoolOptions;
    use uuid::Uuid;

    #[tokio::test]
    async fn revoked_raw_run_query_is_distinct_and_cancellation_aware() {
        let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
            return;
        };
        let pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&database_url)
            .await
            .expect("connect control-plane PostgreSQL");
        sqlx::raw_sql(
            "CREATE TEMP TABLE runs (
                 id uuid PRIMARY KEY,
                 state text NOT NULL,
                 cancel_requested_at timestamptz
             );
             CREATE TEMP TABLE secret_runtime_sessions (
                 id uuid PRIMARY KEY,
                 run_id uuid NOT NULL,
                 status text NOT NULL
             );
             CREATE TEMP TABLE secret_leases (
                 id uuid PRIMARY KEY,
                 session_id uuid NOT NULL,
                 delivery_mode text NOT NULL
             );",
        )
        .execute(&pool)
        .await
        .expect("create focused reconciliation tables");
        let run_id = Uuid::new_v4();
        let session_id = Uuid::new_v4();
        sqlx::query("INSERT INTO runs VALUES ($1, 'running', NULL)")
            .bind(run_id)
            .execute(&pool)
            .await
            .expect("seed running run");
        sqlx::query("INSERT INTO secret_runtime_sessions VALUES ($1, $2, 'revoked')")
            .bind(session_id)
            .bind(run_id)
            .execute(&pool)
            .await
            .expect("seed revoked runtime session");
        for lease_id in [Uuid::new_v4(), Uuid::new_v4()] {
            sqlx::query("INSERT INTO secret_leases VALUES ($1, $2, 'raw')")
                .bind(lease_id)
                .bind(session_id)
                .execute(&pool)
                .await
                .expect("seed duplicate raw lease");
        }
        assert_eq!(revoked_raw_run_ids(&pool).await.unwrap(), vec![run_id]);
        sqlx::query("UPDATE runs SET cancel_requested_at = now() WHERE id = $1")
            .bind(run_id)
            .execute(&pool)
            .await
            .expect("mark run cancelled");
        assert!(revoked_raw_run_ids(&pool).await.unwrap().is_empty());
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

#[derive(Clone)]
struct PreviewArtifact {
    kind: String,
    size_bytes: i64,
    sha256: String,
    storage_key: String,
}

impl From<&RunArtifact> for PreviewArtifact {
    fn from(value: &RunArtifact) -> Self {
        Self {
            kind: value.kind.clone(),
            size_bytes: value.size_bytes,
            sha256: value.sha256.clone(),
            storage_key: value.storage_key.clone(),
        }
    }
}

#[derive(Default)]
struct ResultPreviews {
    patch: Option<String>,
    manifest: Option<String>,
}

fn load_previews(
    root: &Path,
    run_id: Uuid,
    artifacts: &[PreviewArtifact],
) -> Result<ResultPreviews, RunError> {
    let patch = artifacts
        .iter()
        .find(|artifact| artifact.kind == "patch")
        .map(|artifact| read_preview(root, run_id, artifact, "patch"))
        .transpose()?
        .flatten();
    let manifest = artifacts
        .iter()
        .find(|artifact| artifact.kind == "manifest")
        .map(|artifact| read_preview(root, run_id, artifact, "json"))
        .transpose()?
        .flatten();
    Ok(ResultPreviews { patch, manifest })
}

fn read_preview(
    root: &Path,
    run_id: Uuid,
    artifact: &PreviewArtifact,
    extension: &str,
) -> Result<Option<String>, RunError> {
    let recorded_size =
        u64::try_from(artifact.size_bytes).map_err(|_| RunError::PreviewUnavailable)?;
    if recorded_size > MAX_RESULT_PREVIEW_BYTES {
        return Ok(None);
    }
    let expected_key = format!(
        "{run_id}/{}-{}.{}",
        artifact.kind, artifact.sha256, extension
    );
    if artifact.storage_key != expected_key
        || artifact.sha256.len() != 64
        || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RunError::PreviewUnavailable);
    }
    let directory = root.join(run_id.to_string());
    let directory_metadata =
        std::fs::symlink_metadata(&directory).map_err(|_| RunError::PreviewUnavailable)?;
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(RunError::PreviewUnavailable);
    }
    let path = root.join(&artifact.storage_key);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| RunError::PreviewUnavailable)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != recorded_size
    {
        return Err(RunError::PreviewUnavailable);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(0o400_000 | 0o2_000_000)
        .open(path)
        .map_err(|_| RunError::PreviewUnavailable)?;
    let capacity = usize::try_from(recorded_size).map_err(|_| RunError::PreviewUnavailable)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_RESULT_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RunError::PreviewUnavailable)?;
    let maximum_size =
        usize::try_from(MAX_RESULT_PREVIEW_BYTES).map_err(|_| RunError::PreviewUnavailable)?;
    if bytes.len() != capacity || bytes.len() > maximum_size {
        return Err(RunError::PreviewUnavailable);
    }
    let actual_hash = format!("{:x}", Sha256::digest(&bytes));
    if actual_hash != artifact.sha256 {
        return Err(RunError::PreviewUnavailable);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| RunError::PreviewUnavailable)
}

#[derive(FromRow)]
struct EventRow {
    sequence: i64,
    event_type: String,
    payload: Json<Value>,
    occurred_at: OffsetDateTime,
}
#[derive(FromRow)]
struct ControlRow {
    id: Uuid,
    kind: String,
    repository_id: Uuid,
    run_id: Option<Uuid>,
    proposal_id: Option<Uuid>,
    reason: String,
    state: String,
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

fn stable_id(identity: &AuthenticatedIdentity, kind: &str) -> Uuid {
    let mut hash = Sha256::new();
    hash.update(b"hephaestus.control-request.v1\0");
    hash.update(identity.user_id.as_uuid().as_bytes());
    hash.update(identity.idempotency_id.as_uuid().as_bytes());
    hash.update(kind.as_bytes());
    let mut bytes: [u8; 16] = hash.finalize()[..16]
        .try_into()
        .expect("fixed SHA-256 prefix");
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    Uuid::from_bytes(bytes)
}

#[cfg(test)]
mod tests {
    use super::{MAX_RESULT_PREVIEW_BYTES, PreviewArtifact, RunError, load_previews, read_preview};
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::PermissionsExt};
    use uuid::Uuid;

    fn artifact(run_id: Uuid, kind: &str, extension: &str, contents: &[u8]) -> PreviewArtifact {
        let sha256 = format!("{:x}", Sha256::digest(contents));
        PreviewArtifact {
            kind: kind.to_owned(),
            size_bytes: i64::try_from(contents.len()).expect("bounded fixture"),
            storage_key: format!("{run_id}/{kind}-{sha256}.{extension}"),
            sha256,
        }
    }

    #[test]
    fn result_previews_are_run_bound_bounded_and_hash_verified() {
        let temporary = tempfile::tempdir().expect("temporary artifact root");
        fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
            .expect("private artifact root");
        let run_id = Uuid::new_v4();
        let run_directory = temporary.path().join(run_id.to_string());
        fs::create_dir(&run_directory).expect("run artifact directory");
        let patch = b"diff --git a/input.txt b/input.txt\n+changed fixture text\n";
        let manifest = br#"{"entries":[{"path":"input.txt"}]}"#;
        let patch_artifact = artifact(run_id, "patch", "patch", patch);
        let manifest_artifact = artifact(run_id, "manifest", "json", manifest);
        fs::write(temporary.path().join(&patch_artifact.storage_key), patch)
            .expect("patch artifact");
        fs::write(
            temporary.path().join(&manifest_artifact.storage_key),
            manifest,
        )
        .expect("manifest artifact");

        let previews = load_previews(
            temporary.path(),
            run_id,
            &[patch_artifact.clone(), manifest_artifact],
        )
        .expect("verified previews");
        assert_eq!(previews.patch.as_deref(), std::str::from_utf8(patch).ok());
        assert_eq!(
            previews.manifest.as_deref(),
            std::str::from_utf8(manifest).ok()
        );

        let mut wrong_run = patch_artifact.clone();
        wrong_run.storage_key = format!("{}/patch-{}.patch", Uuid::new_v4(), wrong_run.sha256);
        assert!(matches!(
            read_preview(temporary.path(), run_id, &wrong_run, "patch"),
            Err(RunError::PreviewUnavailable)
        ));

        let oversized = PreviewArtifact {
            size_bytes: i64::try_from(MAX_RESULT_PREVIEW_BYTES + 1).expect("preview ceiling"),
            ..patch_artifact
        };
        assert_eq!(
            read_preview(temporary.path(), run_id, &oversized, "patch")
                .expect("oversized previews are omitted"),
            None
        );
    }
}
