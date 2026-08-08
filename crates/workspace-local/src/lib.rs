//! Local exact-commit workspaces and trusted Git result publication.

use agent_config::{AgentConfig, PublicationMode};
use async_trait::async_trait;
use forge_domain::RepositoryId;
use run_domain::{Run, RunKind, RunState};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::ffi::OsStrExt,
    os::unix::fs::{PermissionsExt, symlink},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
};
use uuid::Uuid;
use vm_trait::VmMount;
use workspace_domain::{
    ArtifactId, PreparedWorkspace, PublishedResult, ResultArtifactMetadata, ResultId,
    ResultMetadata, ResultRepository, RunWorkspaceManager, WorkspaceError, WorkspaceId,
    WorkspaceMetadata, WorkspaceMetadataRepository, WorkspaceRepositoryError,
    WorkspaceRequestMetadata,
};

const SOURCE_GUEST_PATH: &str = "/workspace/repo";
const WORK_GUEST_PATH: &str = "/workspace/work";
const DEFAULT_RESULT_MESSAGE: &str = "Hephaestus agent result";

/// Resource limits applied while materializing and importing workspaces.
#[derive(Debug, Clone)]
pub struct WorkspaceLimits {
    /// Maximum number of filesystem entries in one tree.
    pub max_entries: usize,
    /// Maximum bytes in one regular file or symlink target.
    pub max_file_bytes: u64,
    /// Maximum aggregate bytes imported from a workspace.
    pub max_total_bytes: u64,
    /// Maximum bytes stored for a generated patch.
    pub max_patch_bytes: usize,
}

impl Default for WorkspaceLimits {
    fn default() -> Self {
        Self {
            max_entries: 100_000,
            max_file_bytes: 32 * 1024 * 1024,
            max_total_bytes: 256 * 1024 * 1024,
            max_patch_bytes: 64 * 1024 * 1024,
        }
    }
}

/// Configuration for [`LocalWorkspaceManager`].
#[derive(Debug, Clone)]
pub struct LocalWorkspaceConfig {
    /// Root containing active and sealed per-run workspaces.
    pub workspace_root: PathBuf,
    /// Root containing durable content-addressed result artifacts.
    pub artifact_root: PathBuf,
    /// Canonical root containing bare repositories.
    pub repository_root: PathBuf,
    /// Absolute trusted Git executable used only for plumbing commands.
    pub git_binary: PathBuf,
    /// Materialization and import quotas.
    pub limits: WorkspaceLimits,
}

/// Filesystem/Git workspace and result implementation over provider-neutral ports.
#[derive(Clone)]
pub struct LocalWorkspaceManager {
    metadata: Arc<dyn WorkspaceMetadataRepository>,
    results: Arc<dyn ResultRepository>,
    config: LocalWorkspaceConfig,
}

impl LocalWorkspaceManager {
    /// Creates a manager after validating its configured roots.
    ///
    /// # Errors
    ///
    /// Returns an error when a path is relative, overlaps another storage
    /// class, or the Git executable is not an absolute regular file.
    pub fn new(
        metadata: Arc<dyn WorkspaceMetadataRepository>,
        results: Arc<dyn ResultRepository>,
        config: LocalWorkspaceConfig,
    ) -> Result<Self, LocalWorkspaceError> {
        for (name, path) in [
            ("workspace_root", &config.workspace_root),
            ("artifact_root", &config.artifact_root),
            ("repository_root", &config.repository_root),
            ("git_binary", &config.git_binary),
        ] {
            if !path.is_absolute() {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "{name} must be absolute"
                )));
            }
        }
        for (left_name, left, right_name, right) in [
            (
                "workspace_root",
                &config.workspace_root,
                "artifact_root",
                &config.artifact_root,
            ),
            (
                "workspace_root",
                &config.workspace_root,
                "repository_root",
                &config.repository_root,
            ),
            (
                "artifact_root",
                &config.artifact_root,
                "repository_root",
                &config.repository_root,
            ),
        ] {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "{left_name} and {right_name} must not overlap"
                )));
            }
        }
        if config.limits.max_entries == 0
            || config.limits.max_file_bytes == 0
            || config.limits.max_total_bytes == 0
            || config.limits.max_patch_bytes == 0
        {
            return Err(LocalWorkspaceError::Configuration(String::from(
                "workspace limits must be greater than zero",
            )));
        }
        Ok(Self {
            metadata,
            results,
            config,
        })
    }

    /// Creates and canonicalizes workspace and artifact storage roots.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe roots or inaccessible storage.
    pub fn initialize(&mut self) -> Result<(), LocalWorkspaceError> {
        fs::create_dir_all(self.config.workspace_root.join("active")).map_err(io_error)?;
        fs::create_dir_all(self.config.workspace_root.join("sealed")).map_err(io_error)?;
        fs::create_dir_all(&self.config.artifact_root).map_err(io_error)?;
        self.config.workspace_root =
            fs::canonicalize(&self.config.workspace_root).map_err(io_error)?;
        self.config.artifact_root =
            fs::canonicalize(&self.config.artifact_root).map_err(io_error)?;
        self.config.repository_root =
            fs::canonicalize(&self.config.repository_root).map_err(io_error)?;
        for (left_name, left, right_name, right) in [
            (
                "workspace_root",
                &self.config.workspace_root,
                "artifact_root",
                &self.config.artifact_root,
            ),
            (
                "workspace_root",
                &self.config.workspace_root,
                "repository_root",
                &self.config.repository_root,
            ),
            (
                "artifact_root",
                &self.config.artifact_root,
                "repository_root",
                &self.config.repository_root,
            ),
        ] {
            if left.starts_with(right) || right.starts_with(left) {
                return Err(LocalWorkspaceError::Configuration(format!(
                    "canonical {left_name} and {right_name} must not overlap"
                )));
            }
        }
        let git = fs::canonicalize(&self.config.git_binary).map_err(io_error)?;
        if !git.is_file() {
            return Err(LocalWorkspaceError::Configuration(String::from(
                "git_binary must be a regular file",
            )));
        }
        self.config.git_binary = git;
        Ok(())
    }

    fn active_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .workspace_root
            .join("active")
            .join(run_id.to_string())
    }

    fn sealed_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .workspace_root
            .join("sealed")
            .join(run_id.to_string())
    }

    fn repository_path(&self, repository_id: RepositoryId) -> PathBuf {
        self.config
            .repository_root
            .join(format!("{repository_id}.git"))
    }

    async fn request(&self, run: &Run) -> Result<Option<RunRequest>, LocalWorkspaceError> {
        let row = self
            .metadata
            .request(run.command_id.as_uuid())
            .await
            .map_err(repository)?;
        row.map(TryInto::try_into).transpose()
    }

    async fn existing_result(
        &self,
        run_id: RunId,
    ) -> Result<Option<PublishedResult>, LocalWorkspaceError> {
        let row = self.results.completed(run_id).await.map_err(repository)?;
        row.map(published_result).transpose()
    }
}

#[async_trait]
impl RunWorkspaceManager for LocalWorkspaceManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedWorkspace, WorkspaceError> {
        self.prepare_run(run)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn finalize(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, WorkspaceError> {
        self.finalize_run(run, message)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn abandon(&self, run_id: RunId) -> Result<(), WorkspaceError> {
        self.abandon_run(run_id)
            .await
            .map_err(WorkspaceError::operation)
    }

    async fn recover(&self) -> Result<usize, WorkspaceError> {
        self.recover_incomplete()
            .await
            .map_err(WorkspaceError::operation)
    }
}

impl LocalWorkspaceManager {
    // Keeping the ordered materialization state changes together makes cleanup
    // and crash boundaries directly auditable.
    #[allow(clippy::too_many_lines)]
    async fn prepare_run(&self, run: &Run) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        let Some(request) = self.request(run).await? else {
            return Ok(PreparedWorkspace::disabled());
        };
        if !request.config.workspace.mount {
            return Ok(PreparedWorkspace::disabled());
        }
        validate_mount_policy(&request.config)?;

        if let Some(row) = self.metadata.workspace(run.id).await.map_err(repository)? {
            if row.state == "active" {
                return self.prepared_from_row(&row);
            }
            return Err(LocalWorkspaceError::State(format!(
                "workspace for run {} is already {}",
                run.id, row.state
            )));
        }

        let workspace_id = WorkspaceId::new();
        let active_path = self.active_path(run.id);
        let sealed_path = self.sealed_path(run.id);
        let active_text = utf8_path(&active_path)?;
        let sealed_text = utf8_path(&sealed_path)?;
        self.metadata
            .insert_preparing(
                &WorkspaceMetadata {
                    id: workspace_id.as_uuid(),
                    state: String::from("preparing"),
                    active_path: active_text.clone(),
                    sealed_path: sealed_text.clone(),
                    input_commit: Some(request.commit.clone()),
                },
                request.repository_id.as_uuid(),
                &request.commit,
                run.id,
            )
            .await
            .map_err(repository)?;

        let temporary_path = self
            .config
            .workspace_root
            .join("active")
            .join(format!("{}.{}", run.id, workspace_id));
        let repository_path = self.repository_path(request.repository_id);
        let config = self.config.clone();
        let commit = request.commit.clone();
        let failed_temporary_path = temporary_path.clone();
        let materialized = tokio::task::spawn_blocking(move || {
            materialize(
                &config,
                &repository_path,
                &commit,
                &temporary_path,
                &active_path,
            )
        })
        .await
        .map_err(join_error)?;
        let materialized = match materialized {
            Ok(value) => value,
            Err(error) => {
                if failed_temporary_path
                    .join(".hephaestus-workspace")
                    .is_file()
                {
                    remove_owned_workspace(&self.config, &failed_temporary_path, "active")?;
                }
                self.metadata
                    .mark_materialization_failed(run.id, &error.to_string())
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
        };
        self.metadata
            .mark_active(
                run.id,
                &materialized.tree,
                &materialized.manifest_hash,
                serde_json::json!({
                    "workspace_id": workspace_id.to_string(),
                    "repository_id": request.repository_id,
                    "input_commit": request.commit,
                    "input_tree": materialized.tree,
                    "materialization_hash": materialized.manifest_hash,
                    "source_mount": SOURCE_GUEST_PATH,
                    "work_mount": WORK_GUEST_PATH,
                }),
            )
            .await
            .map_err(repository)?;
        self.prepared(workspace_id, &self.active_path(run.id))
    }

    fn prepared_from_row(
        &self,
        row: &WorkspaceMetadata,
    ) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        let active = PathBuf::from(&row.active_path);
        let expected =
            self.config
                .workspace_root
                .join("active")
                .join(active.file_name().ok_or_else(|| {
                    LocalWorkspaceError::UnsafePath(String::from(
                        "active workspace has no file name",
                    ))
                })?);
        if active != expected
            || row.sealed_path != utf8_path(&self.sealed_path_from_active(&active))?
        {
            return Err(LocalWorkspaceError::UnsafePath(String::from(
                "stored workspace paths do not use the canonical layout",
            )));
        }
        self.prepared(WorkspaceId::from_uuid(row.id), &active)
    }

    fn sealed_path_from_active(&self, active: &Path) -> PathBuf {
        self.config
            .workspace_root
            .join("sealed")
            .join(active.file_name().unwrap_or_else(|| OsStr::new("invalid")))
    }

    fn prepared(
        &self,
        workspace_id: WorkspaceId,
        active: &Path,
    ) -> Result<PreparedWorkspace, LocalWorkspaceError> {
        ensure_workspace_path(&self.config, active, "active")?;
        Ok(PreparedWorkspace {
            id: Some(workspace_id),
            mounts: vec![
                VmMount {
                    tag: String::from("repository-source"),
                    host_path: active.join("source"),
                    guest_path: PathBuf::from(SOURCE_GUEST_PATH),
                    read_only: true,
                },
                VmMount {
                    tag: String::from("repository-work"),
                    host_path: active.join("work"),
                    guest_path: PathBuf::from(WORK_GUEST_PATH),
                    read_only: false,
                },
            ],
        })
    }

    // The seal, import, database, artifact, and Git CAS order is security
    // sensitive and intentionally visible as one state-machine operation.
    #[allow(clippy::too_many_lines)]
    // Finalization must keep validation and every durable filesystem/database
    // transition in one ordered cleanup path.
    #[allow(clippy::cognitive_complexity)]
    async fn finalize_run(
        &self,
        run: &Run,
        message: &str,
    ) -> Result<Option<PublishedResult>, LocalWorkspaceError> {
        if let Some(result) = self.existing_result(run.id).await? {
            self.cleanup_completed_workspace(run.id).await?;
            return Ok(Some(result));
        }
        let Some(request) = self.request(run).await? else {
            return Ok(None);
        };
        if !request.config.workspace.mount {
            return Ok(None);
        }
        validate_mount_policy(&request.config)?;
        let message = validate_message(message)?;
        let workspace = self
            .metadata
            .workspace(run.id)
            .await
            .map_err(repository)?
            .ok_or_else(|| {
                LocalWorkspaceError::State(String::from("workspace metadata is missing"))
            })?;
        let active = PathBuf::from(&workspace.active_path);
        let sealed = PathBuf::from(&workspace.sealed_path);
        ensure_workspace_path(&self.config, &active, "active")?;
        ensure_workspace_path(&self.config, &sealed, "sealed")?;
        let result_ref = format!("refs/heads/hephaestus/{}/{}", request.instance_id, run.id);
        self.results
            .insert_pending(
                run.id,
                request.repository_id.as_uuid(),
                request.instance_id.as_uuid(),
                run.instance_revision_id.as_uuid(),
                run.release_id.as_uuid(),
                run.release_agent_id.as_uuid(),
                workspace.input_commit.as_deref().unwrap_or_default(),
                &result_ref,
                message,
            )
            .await
            .map_err(repository)?;

        if workspace.state == "active" {
            self.metadata
                .set_state(run.id, "finalize_requested")
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    run.id,
                    "result.finalize_requested",
                    serde_json::json!({"message": message}),
                )
                .await
                .map_err(repository)?;
        }
        if matches!(
            workspace.state.as_str(),
            "active" | "finalize_requested" | "seal_failed"
        ) {
            if let Err(error) = seal_workspace(&active, &sealed) {
                self.metadata
                    .mark_failed(run.id, "seal_failed", &error.to_string())
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
            self.metadata
                .set_state(run.id, "sealed")
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    run.id,
                    "result.workspace_sealed",
                    serde_json::json!({"workspace_id": workspace.id}),
                )
                .await
                .map_err(repository)?;
        }

        self.metadata
            .set_state(run.id, "importing")
            .await
            .map_err(repository)?;

        let repository_path = self.repository_path(request.repository_id);
        let config = self.config.clone();
        let sealed_for_import = sealed.clone();
        let input_commit = workspace.input_commit.clone();
        let message_for_import = message.to_owned();
        let declared_files = request.config.results.declared_files.clone();
        let timestamp = run.created_at.unix_timestamp();
        let result_run_id = run.id;
        let import = ImportRequest {
            input_commit: input_commit.unwrap_or_default(),
            message: message_for_import,
            timestamp,
            declared_paths: declared_files,
            repository_id: request.repository_id,
            run_id: result_run_id,
        };
        let imported = tokio::task::spawn_blocking(move || {
            import_result(&config, &repository_path, &sealed_for_import, &import)
        })
        .await
        .map_err(join_error)?;
        let imported = match imported {
            Ok(imported) => imported,
            Err(error) => {
                self.results
                    .reject(run.id, &error.to_string())
                    .await
                    .map_err(repository)?;
                self.metadata
                    .mark_failed(run.id, "import_rejected", &error.to_string())
                    .await
                    .map_err(repository)?;
                self.metadata
                    .event(
                        run.id,
                        "result.import_rejected",
                        serde_json::json!({"message": error.to_string()}),
                    )
                    .await
                    .map_err(repository)?;
                return Err(error);
            }
        };

        let persisted_id = self.results.id_for_run(run.id).await.map_err(repository)?;
        let (logs, exit) = self.results.vm_logs(run.id).await.map_err(repository)?;
        let artifacts = persist_artifacts(
            &imported,
            &logs,
            &exit.unwrap_or(serde_json::Value::Null),
            run.id,
            &self.config.artifact_root,
        )?;
        self.results
            .persist_prepared(
                persisted_id,
                run.id,
                &imported.tree,
                &imported.commit,
                &imported.manifest_hash,
                &artifacts,
            )
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.import_prepared",
                serde_json::json!({
                    "result_id": persisted_id.to_string(),
                    "result_tree": imported.tree,
                    "result_commit": imported.commit,
                    "artifact_manifest_hash": imported.manifest_hash,
                }),
            )
            .await
            .map_err(repository)?;

        cas_publish_ref(
            &self.config,
            &self.repository_path(request.repository_id),
            &result_ref,
            &imported.commit,
        )?;
        self.results
            .mark_ref_published(persisted_id, &imported.commit)
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.ref_published",
                serde_json::json!({"result_ref": result_ref, "result_commit": imported.commit}),
            )
            .await
            .map_err(repository)?;
        self.results
            .mark_completed(persisted_id)
            .await
            .map_err(repository)?;
        self.metadata
            .event(
                run.id,
                "result.completed",
                serde_json::json!({"result_id": persisted_id.to_string()}),
            )
            .await
            .map_err(repository)?;
        self.cleanup_completed_workspace(run.id).await?;
        Ok(Some(PublishedResult {
            id: persisted_id,
            result_ref,
            result_commit: imported.commit,
            result_tree: imported.tree,
        }))
    }

    async fn abandon_run(&self, run_id: RunId) -> Result<(), LocalWorkspaceError> {
        let row = self.metadata.workspace(run_id).await.map_err(repository)?;
        let Some(row) = row else {
            return Ok(());
        };
        if row.state == "cleaned" {
            return Ok(());
        }
        let active = PathBuf::from(row.active_path);
        if active.exists() {
            remove_owned_workspace(&self.config, &active, "active")?;
        }
        let sealed = PathBuf::from(row.sealed_path);
        if sealed.exists() {
            remove_owned_workspace(&self.config, &sealed, "sealed")?;
        }
        self.metadata
            .set_state(run_id, "abandoned")
            .await
            .map_err(repository)?;
        self.metadata
            .event(run_id, "workspace.abandoned", serde_json::json!({}))
            .await
            .map_err(repository)
    }

    async fn recover_incomplete(&self) -> Result<usize, LocalWorkspaceError> {
        let pending = self.results.pending().await.map_err(repository)?;
        let mut recovered = 0;
        for row in pending {
            let run = Run {
                id: RunId::from_uuid(row.run_id),
                instance_id: AgentInstanceId::from_uuid(row.instance_id),
                instance_revision_id: AgentInstanceRevisionId::from_uuid(row.instance_revision_id),
                release_id: ReleaseId::from_uuid(row.release_id),
                release_agent_id: ReleaseAgentId::from_uuid(row.release_agent_id),
                attachment_id: row.attachment_id.map(AgentAttachmentId::from_uuid),
                kind: parse_run_kind(&row.run_kind)?,
                command_id: CommandId::from_uuid(row.command_id),
                requires_state: row.requires_state,
                volume_id: None,
                lease_id: None,
                vm_id: None,
                state: RunState::Running,
                outcome: None,
                exit: None,
                failure: None,
                cancel_requested_at: None,
                created_at: row.created_at,
                updated_at: row.created_at,
                state_version: 0,
            };
            self.finalize_run(&run, &row.message).await?;
            recovered += 1;
        }
        let rows = self.results.prepared().await.map_err(repository)?;
        for row in rows {
            let Some(commit) = row.result_commit else {
                continue;
            };
            let repository_path = self.repository_path(RepositoryId::from_uuid(row.repository_id));
            cas_publish_ref(&self.config, &repository_path, &row.result_ref, &commit)?;
            self.results
                .mark_completed(ResultId::from_uuid(row.id))
                .await
                .map_err(repository)?;
            self.metadata
                .event(
                    RunId::from_uuid(row.run_id),
                    "result.completed",
                    serde_json::json!({"recovered": true, "result_commit": commit}),
                )
                .await
                .map_err(repository)?;
            self.cleanup_completed_workspace(RunId::from_uuid(row.run_id))
                .await?;
            recovered += 1;
        }
        Ok(recovered)
    }

    async fn cleanup_completed_workspace(&self, run_id: RunId) -> Result<(), LocalWorkspaceError> {
        let workspace = self.metadata.workspace(run_id).await.map_err(repository)?;
        let Some(workspace) = workspace else {
            return Ok(());
        };
        if workspace.state == "cleaned" {
            return Ok(());
        }
        for (path, class) in [
            (PathBuf::from(workspace.active_path), "active"),
            (PathBuf::from(workspace.sealed_path), "sealed"),
        ] {
            if path.exists() {
                remove_owned_workspace(&self.config, &path, class)?;
            }
        }
        self.metadata
            .mark_cleaned(run_id)
            .await
            .map_err(repository)?;
        self.metadata
            .event(run_id, "workspace.cleaned", serde_json::json!({}))
            .await
            .map_err(repository)
    }
}

struct RunRequest {
    repository_id: RepositoryId,
    commit: String,
    instance_id: AgentInstanceId,
    config: AgentConfig,
}

impl TryFrom<WorkspaceRequestMetadata> for RunRequest {
    type Error = LocalWorkspaceError;

    fn try_from(row: WorkspaceRequestMetadata) -> Result<Self, Self::Error> {
        Ok(Self {
            repository_id: RepositoryId::from_uuid(row.repository_id),
            commit: row.commit_sha,
            instance_id: AgentInstanceId::from_uuid(row.instance_id),
            config: serde_json::from_value(row.configuration).map_err(serialization)?,
        })
    }
}
fn published_result(row: ResultMetadata) -> Result<PublishedResult, LocalWorkspaceError> {
    Ok(PublishedResult {
        id: ResultId::from_uuid(row.id),
        result_ref: row.result_ref,
        result_commit: row.result_commit.ok_or_else(|| {
            LocalWorkspaceError::State(String::from("completed result has no commit"))
        })?,
        result_tree: row.result_tree.ok_or_else(|| {
            LocalWorkspaceError::State(String::from("completed result has no tree"))
        })?,
    })
}

struct Materialized {
    tree: String,
    manifest_hash: String,
}

fn parse_run_kind(value: &str) -> Result<RunKind, LocalWorkspaceError> {
    match value {
        "normal" => Ok(RunKind::Normal),
        "update" => Ok(RunKind::Update),
        _ => Err(LocalWorkspaceError::State(String::from(
            "stored run kind is invalid",
        ))),
    }
}

struct Imported {
    tree: String,
    commit: String,
    manifest: Vec<u8>,
    manifest_hash: String,
    patch: Vec<u8>,
    declared_files: Vec<DeclaredFile>,
}

struct ImportRequest {
    input_commit: String,
    message: String,
    timestamp: i64,
    declared_paths: Vec<String>,
    repository_id: RepositoryId,
    run_id: RunId,
}

struct DeclaredFile {
    path: String,
    mode: u32,
    bytes: Vec<u8>,
    sha256: String,
}

#[derive(Serialize)]
struct Manifest {
    version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input_commit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_tree: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    result_commit: Option<String>,
    entries: Vec<ManifestEntry>,
}

#[derive(Serialize)]
struct ManifestEntry {
    path: String,
    kind: &'static str,
    mode: u32,
    size: u64,
    sha256: String,
}

fn validate_mount_policy(config: &AgentConfig) -> Result<(), LocalWorkspaceError> {
    if config.publication.mode != PublicationMode::Proposal
        || config.publication.mode.permits_git_write_remote()
    {
        return Err(LocalWorkspaceError::Configuration(String::from(
            "controlled workspaces are available only to proposal-mode releases",
        )));
    }
    if config.workspace.path != SOURCE_GUEST_PATH || !config.workspace.read_only {
        return Err(LocalWorkspaceError::Configuration(format!(
            "workspace source mount must request read-only {SOURCE_GUEST_PATH}"
        )));
    }
    Ok(())
}

fn validate_message(message: &str) -> Result<&str, LocalWorkspaceError> {
    let message = if message.trim().is_empty() {
        DEFAULT_RESULT_MESSAGE
    } else {
        message
    };
    if message.len() > 4096 || message.contains('\0') {
        return Err(LocalWorkspaceError::InvalidResult(String::from(
            "result message must contain at most 4096 UTF-8 bytes and no NUL",
        )));
    }
    Ok(message)
}

fn materialize(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    commit: &str,
    temporary: &Path,
    active: &Path,
) -> Result<Materialized, LocalWorkspaceError> {
    validate_repository(config, repository)?;
    ensure_workspace_path(config, temporary, "active")?;
    ensure_workspace_path(config, active, "active")?;
    if temporary.exists() || active.exists() {
        return Err(LocalWorkspaceError::State(String::from(
            "workspace path already exists",
        )));
    }
    fs::create_dir(temporary).map_err(io_error)?;
    let owner = active
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("active owner is invalid")))?;
    write_owner_marker(temporary, owner)?;
    let source = temporary.join("source");
    let work = temporary.join("work");
    fs::create_dir(&source).map_err(io_error)?;
    let entries = git_ls_tree(config, repository, commit)?;
    let mut manifest = Manifest {
        version: 1,
        repository_id: None,
        run_id: None,
        input_commit: None,
        result_tree: None,
        result_commit: None,
        entries: Vec::with_capacity(entries.len()),
    };
    let mut total = 0_u64;
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if entry.kind != "blob" || !matches!(entry.mode, 0o100_644 | 0o100_755 | 0o120_000) {
            return Err(LocalWorkspaceError::InvalidSource(format!(
                "unsupported Git entry {} {:o} {}",
                entry.kind, entry.mode, entry.path
            )));
        }
        let bytes = git_output(
            config,
            repository,
            &["cat-file", "blob", &entry.object_id],
            None,
            &[],
        )?;
        enforce_bytes(config, &mut total, bytes.len() as u64)?;
        let destination = source.join(&entry.path);
        let parent = destination.parent().ok_or_else(|| {
            LocalWorkspaceError::UnsafePath(String::from("source entry has no parent"))
        })?;
        fs::create_dir_all(parent).map_err(io_error)?;
        if entry.mode == 0o120_000 {
            let target = std::str::from_utf8(&bytes).map_err(|_| {
                LocalWorkspaceError::InvalidSource(format!(
                    "symlink target for {} is not UTF-8",
                    entry.path
                ))
            })?;
            validate_symlink_target(target)?;
            symlink(target, &destination).map_err(io_error)?;
        } else {
            write_new_file(&destination, &bytes, entry.mode & 0o111 != 0)?;
        }
        manifest.entries.push(ManifestEntry {
            path: entry.path,
            kind: if entry.mode == 0o120_000 {
                "symlink"
            } else {
                "file"
            },
            mode: entry.mode,
            size: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
    }
    copy_tree(&source, &work)?;
    make_source_read_only(&source)?;
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(serialization)?;
    let manifest_hash = sha256(&manifest_bytes);
    fsync_tree(temporary)?;
    fs::rename(temporary, active).map_err(io_error)?;
    sync_directory(
        active
            .parent()
            .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("active root missing")))?,
    )?;
    let mut treeish = commit.to_owned();
    treeish.push('^');
    treeish.push('{');
    treeish.push_str("tree");
    treeish.push('}');
    let tree = git_text(config, repository, &["rev-parse", &treeish], None, &[])?;
    Ok(Materialized {
        tree,
        manifest_hash,
    })
}

struct GitTreeEntry {
    mode: u32,
    kind: String,
    object_id: String,
    path: String,
}

fn git_ls_tree(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    commit: &str,
) -> Result<Vec<GitTreeEntry>, LocalWorkspaceError> {
    let output = git_output(
        config,
        repository,
        &["ls-tree", "-rz", "-r", "--full-tree", commit],
        None,
        &[],
    )?;
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                LocalWorkspaceError::Git(String::from("ls-tree record has no path separator"))
            })?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| LocalWorkspaceError::Git(String::from("ls-tree metadata is not UTF-8")))?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| LocalWorkspaceError::InvalidSource(String::from("non-UTF-8 Git path")))?;
        let mut fields = metadata.split_ascii_whitespace();
        let mode = u32::from_str_radix(
            fields.next().ok_or_else(|| {
                LocalWorkspaceError::Git(String::from("ls-tree record has no mode"))
            })?,
            8,
        )
        .map_err(|error| LocalWorkspaceError::Git(error.to_string()))?;
        let kind = fields
            .next()
            .ok_or_else(|| LocalWorkspaceError::Git(String::from("ls-tree record has no type")))?;
        let object_id = fields.next().ok_or_else(|| {
            LocalWorkspaceError::Git(String::from("ls-tree record has no object ID"))
        })?;
        entries.push(GitTreeEntry {
            mode,
            kind: kind.to_owned(),
            object_id: object_id.to_owned(),
            path: path.to_owned(),
        });
    }
    if entries.len() > config.limits.max_entries {
        return Err(LocalWorkspaceError::Quota(String::from(
            "source tree exceeds entry limit",
        )));
    }
    Ok(entries)
}

fn import_result(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    sealed: &Path,
    request: &ImportRequest,
) -> Result<Imported, LocalWorkspaceError> {
    validate_repository(config, repository)?;
    ensure_workspace_path(config, sealed, "sealed")?;
    let work = sealed.join("work");
    let mut manifest = Manifest {
        version: 1,
        repository_id: Some(request.repository_id.to_string()),
        run_id: Some(request.run_id.to_string()),
        input_commit: Some(request.input_commit.clone()),
        result_tree: None,
        result_commit: None,
        entries: Vec::new(),
    };
    let mut counters = ImportCounters::default();
    let tree = import_directory(
        config,
        repository,
        &work,
        Path::new(""),
        &mut manifest,
        &mut counters,
    )?;
    let commit = git_text(
        config,
        repository,
        &["commit-tree", &tree, "-p", &request.input_commit],
        Some(request.message.as_bytes()),
        &[
            ("GIT_AUTHOR_NAME", "Hephaestus Agent"),
            ("GIT_AUTHOR_EMAIL", "agent@hephaestus.invalid"),
            ("GIT_COMMITTER_NAME", "Hephaestus Result Publisher"),
            ("GIT_COMMITTER_EMAIL", "result@hephaestus.invalid"),
            ("GIT_AUTHOR_DATE", &format!("@{} +0000", request.timestamp)),
            (
                "GIT_COMMITTER_DATE",
                &format!("@{} +0000", request.timestamp),
            ),
        ],
    )?;
    manifest.result_tree = Some(tree.clone());
    manifest.result_commit = Some(commit.clone());
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(serialization)?;
    let manifest_hash = sha256(&manifest_bytes);
    let patch = git_output(
        config,
        repository,
        &[
            "diff-tree",
            "-p",
            "--binary",
            "--no-ext-diff",
            &request.input_commit,
            &commit,
        ],
        None,
        &[],
    )?;
    if patch.len() > config.limits.max_patch_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "generated result patch exceeds configured limit",
        )));
    }
    let mut declared_files = Vec::with_capacity(request.declared_paths.len());
    for declared_path in &request.declared_paths {
        let (declared_host_path, metadata) = declared_regular_file(&work, declared_path)?;
        let bytes = fs::read(&declared_host_path).map_err(io_error)?;
        declared_files.push(DeclaredFile {
            path: declared_path.clone(),
            mode: if metadata.permissions().mode() & 0o111 != 0 {
                0o100_755
            } else {
                0o100_644
            },
            sha256: sha256(&bytes),
            bytes,
        });
    }
    Ok(Imported {
        tree,
        commit,
        manifest: manifest_bytes,
        manifest_hash,
        patch,
        declared_files,
    })
}

fn declared_regular_file(
    work: &Path,
    declared_path: &str,
) -> Result<(PathBuf, fs::Metadata), LocalWorkspaceError> {
    validate_relative_path(declared_path)?;
    let components = Path::new(declared_path).components().collect::<Vec<_>>();
    let mut current = work.to_owned();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(LocalWorkspaceError::UnsafePath(format!(
                "unsafe declared result path {declared_path:?}"
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            LocalWorkspaceError::InvalidResult(format!(
                "declared result file {declared_path:?} cannot be inspected: {error}"
            ))
        })?;
        let is_final = index + 1 == components.len();
        if is_final && metadata.file_type().is_file() {
            return Ok((current, metadata));
        }
        if !is_final && metadata.file_type().is_dir() {
            continue;
        }
        return Err(LocalWorkspaceError::InvalidResult(format!(
            "declared result path {declared_path:?} contains a symlink or non-directory parent"
        )));
    }
    Err(LocalWorkspaceError::InvalidResult(format!(
        "declared result path {declared_path:?} is not a regular file"
    )))
}

#[derive(Default)]
struct ImportCounters {
    entries: usize,
    bytes: u64,
}

// Recursive import keeps the checks adjacent to each supported file kind.
#[allow(clippy::too_many_lines)]
fn import_directory(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    directory: &Path,
    relative: &Path,
    manifest: &mut Manifest,
    counters: &mut ImportCounters,
) -> Result<String, LocalWorkspaceError> {
    let mut children = fs::read_dir(directory)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    let mut tree_entries = Vec::with_capacity(children.len());
    for child in children {
        counters.entries += 1;
        if counters.entries > config.limits.max_entries {
            return Err(LocalWorkspaceError::Quota(String::from(
                "result tree exceeds entry limit",
            )));
        }
        let name = child.file_name().into_string().map_err(|_| {
            LocalWorkspaceError::InvalidResult(String::from("result path is not UTF-8"))
        })?;
        if name.eq_ignore_ascii_case(".git") {
            return Err(LocalWorkspaceError::InvalidResult(String::from(
                ".git paths are forbidden in result workspaces",
            )));
        }
        validate_name(&name)?;
        let child_relative = relative.join(&name);
        let path = child.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            let object_id = import_directory(
                config,
                repository,
                &path,
                &child_relative,
                manifest,
                counters,
            )?;
            tree_entries.push(TreeObject {
                mode: 0o040_000,
                kind: "tree",
                object_id,
                name,
            });
        } else if file_type.is_file() {
            let size = metadata.len();
            enforce_bytes(config, &mut counters.bytes, size)?;
            let bytes = fs::read(&path).map_err(io_error)?;
            let executable = metadata.permissions().mode() & 0o111 != 0;
            let mode = if executable { 0o100_755 } else { 0o100_644 };
            let object_id = git_text(
                config,
                repository,
                &["hash-object", "-w", "--stdin"],
                Some(&bytes),
                &[],
            )?;
            manifest.entries.push(ManifestEntry {
                path: utf8_path(&child_relative)?,
                kind: "file",
                mode,
                size,
                sha256: sha256(&bytes),
            });
            tree_entries.push(TreeObject {
                mode,
                kind: "blob",
                object_id,
                name,
            });
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path).map_err(io_error)?;
            let target_bytes = target.as_os_str().as_bytes();
            enforce_bytes(config, &mut counters.bytes, target_bytes.len() as u64)?;
            let target_text = std::str::from_utf8(target_bytes).map_err(|_| {
                LocalWorkspaceError::InvalidResult(String::from("symlink target is not UTF-8"))
            })?;
            validate_symlink_target(target_text)?;
            let object_id = git_text(
                config,
                repository,
                &["hash-object", "-w", "--stdin"],
                Some(target_bytes),
                &[],
            )?;
            manifest.entries.push(ManifestEntry {
                path: utf8_path(&child_relative)?,
                kind: "symlink",
                mode: 0o120_000,
                size: target_bytes.len() as u64,
                sha256: sha256(target_bytes),
            });
            tree_entries.push(TreeObject {
                mode: 0o120_000,
                kind: "blob",
                object_id,
                name,
            });
        } else {
            return Err(LocalWorkspaceError::InvalidResult(format!(
                "unsupported filesystem object at {}",
                child_relative.display()
            )));
        }
    }
    let mut input = Vec::new();
    for entry in tree_entries {
        write!(
            input,
            "{:06o} {} {}\t{}\0",
            entry.mode, entry.kind, entry.object_id, entry.name
        )
        .map_err(io_error)?;
    }
    git_text(config, repository, &["mktree", "-z"], Some(&input), &[])
}

struct TreeObject {
    mode: u32,
    kind: &'static str,
    object_id: String,
    name: String,
}

#[allow(clippy::too_many_lines)] // Artifact metadata mirrors the durable result schema explicitly.
fn persist_artifacts(
    imported: &Imported,
    logs: &[serde_json::Value],
    exit: &serde_json::Value,
    run_id: RunId,
    artifact_root: &Path,
) -> Result<Vec<ResultArtifactMetadata>, LocalWorkspaceError> {
    let logs = serde_json::to_vec(logs).map_err(serialization)?;
    let exit = serde_json::to_vec(exit).map_err(serialization)?;
    let artifact_directory = artifact_root.join(run_id.to_string());
    fs::create_dir_all(&artifact_directory).map_err(io_error)?;
    let manifest_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "manifest",
        &imported.manifest_hash,
        "json",
        &imported.manifest,
    )?;
    let patch_hash = sha256(&imported.patch);
    let patch_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "patch",
        &patch_hash,
        "patch",
        &imported.patch,
    )?;
    let logs_hash = sha256(&logs);
    let logs_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "logs",
        &logs_hash,
        "json",
        &logs,
    )?;
    let exit_hash = sha256(&exit);
    let exit_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "exit",
        &exit_hash,
        "json",
        &exit,
    )?;
    let mut artifacts = vec![
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("manifest"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(imported.manifest.len()).map_err(integer_error)?,
            sha256: imported.manifest_hash.clone(),
            storage_key: manifest_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("logs"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(logs.len()).map_err(integer_error)?,
            sha256: logs_hash,
            storage_key: logs_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("exit"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(exit.len()).map_err(integer_error)?,
            sha256: exit_hash,
            storage_key: exit_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("patch"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("text/x-diff"),
            size_bytes: i64::try_from(imported.patch.len()).map_err(integer_error)?,
            sha256: patch_hash,
            storage_key: patch_key,
        },
    ];
    for declared in &imported.declared_files {
        let key = store_artifact(
            artifact_root,
            &artifact_directory,
            "declared",
            &declared.sha256,
            "bin",
            &declared.bytes,
        )?;
        artifacts.push(ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("declared_file"),
            path: declared.path.clone(),
            git_mode: Some(i32::try_from(declared.mode).map_err(integer_error)?),
            media_type: String::from("application/octet-stream"),
            size_bytes: i64::try_from(declared.bytes.len()).map_err(integer_error)?,
            sha256: declared.sha256.clone(),
            storage_key: key,
        });
    }
    Ok(artifacts)
}

fn store_artifact(
    artifact_root: &Path,
    directory: &Path,
    kind: &str,
    hash: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<String, LocalWorkspaceError> {
    let name = format!("{kind}-{hash}.{extension}");
    let destination = directory.join(&name);
    if !destination.exists() {
        let temporary = directory.join(format!(".{name}.tmp"));
        write_new_file(&temporary, bytes, false)?;
        fs::rename(&temporary, &destination).map_err(io_error)?;
        sync_directory(directory)?;
    }
    let relative = destination.strip_prefix(artifact_root).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("artifact escaped artifact root"))
    })?;
    utf8_path(relative)
}

fn cas_publish_ref(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    result_ref: &str,
    commit: &str,
) -> Result<(), LocalWorkspaceError> {
    let existing = git_optional_text(config, repository, &["rev-parse", "--verify", result_ref])?;
    if let Some(existing) = existing {
        if existing == commit {
            return Ok(());
        }
        return Err(LocalWorkspaceError::Integrity(format!(
            "result ref {result_ref} points to {existing}, expected {commit}"
        )));
    }
    let zero = "0".repeat(commit.len());
    git_output(
        config,
        repository,
        &["update-ref", result_ref, commit, &zero],
        None,
        &[],
    )?;
    Ok(())
}

fn git_optional_text(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
) -> Result<Option<String>, LocalWorkspaceError> {
    let mut command = git_command(config, repository, arguments);
    let output = command.output().map_err(io_error)?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(Some(text.trim().to_owned()))
}

fn git_text(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Result<String, LocalWorkspaceError> {
    let bytes = git_output(config, repository, arguments, input, environment)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(text.trim().to_owned())
}

fn git_output(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Result<Vec<u8>, LocalWorkspaceError> {
    let mut command = git_command(config, repository, arguments);
    for (key, value) in environment {
        command.env(key, value);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(io_error)?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| LocalWorkspaceError::Git(String::from("Git stdin was not piped")))?
            .write_all(input)
            .map_err(io_error)?;
    }
    let output = child.wait_with_output().map_err(io_error)?;
    if !output.status.success() {
        return Err(LocalWorkspaceError::Git(format!(
            "Git {:?} failed: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn git_command(config: &LocalWorkspaceConfig, repository: &Path, arguments: &[&str]) -> Command {
    let mut command = Command::new(&config.git_binary);
    command
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments);
    command
}

fn validate_repository(
    config: &LocalWorkspaceConfig,
    repository: &Path,
) -> Result<(), LocalWorkspaceError> {
    let parent = repository.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("repository path has no parent"))
    })?;
    let metadata = fs::symlink_metadata(repository).map_err(io_error)?;
    if parent != config.repository_root
        || repository.extension() != Some(OsStr::new("git"))
        || !metadata.file_type().is_dir()
    {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "repository path is outside the canonical layout",
        )));
    }
    Ok(())
}

fn validate_relative_path(path: &str) -> Result<(), LocalWorkspaceError> {
    let path = Path::new(path);
    if path.is_absolute() || path.components().next().is_none() {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "invalid repository path {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.eq_ignore_ascii_case(OsStr::new(".git")) => {}
            _ => {
                return Err(LocalWorkspaceError::UnsafePath(format!(
                    "unsafe repository path {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

fn validate_name(name: &str) -> Result<(), LocalWorkspaceError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "unsafe workspace entry {name:?}"
        )));
    }
    Ok(())
}

fn validate_symlink_target(target: &str) -> Result<(), LocalWorkspaceError> {
    let path = Path::new(target);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "unsafe symlink target {target:?}"
        )));
    }
    Ok(())
}

fn enforce_bytes(
    config: &LocalWorkspaceConfig,
    total: &mut u64,
    size: u64,
) -> Result<(), LocalWorkspaceError> {
    if size > config.limits.max_file_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "file exceeds configured byte limit",
        )));
    }
    *total = total
        .checked_add(size)
        .ok_or_else(|| LocalWorkspaceError::Quota(String::from("workspace size overflow")))?;
    if *total > config.limits.max_total_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "workspace exceeds configured aggregate byte limit",
        )));
    }
    Ok(())
}

fn ensure_workspace_path(
    config: &LocalWorkspaceConfig,
    path: &Path,
    class: &str,
) -> Result<(), LocalWorkspaceError> {
    let expected_parent = config.workspace_root.join(class);
    if path.parent() != Some(expected_parent.as_path()) || path.file_name().is_none() {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "workspace path {} is outside {class}",
            path.display()
        )));
    }
    Ok(())
}

fn seal_workspace(active: &Path, sealed: &Path) -> Result<(), LocalWorkspaceError> {
    if sealed.exists() {
        if active.exists() {
            return Err(LocalWorkspaceError::Integrity(String::from(
                "active and sealed workspace both exist",
            )));
        }
        return Ok(());
    }
    fsync_tree(active)?;
    let active_parent = active.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("active workspace root missing"))
    })?;
    let sealed_parent = sealed
        .parent()
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("sealed root missing")))?;
    fs::rename(active, sealed).map_err(io_error)?;
    sync_directory(active_parent)?;
    sync_directory(sealed_parent)
}

fn remove_owned_workspace(
    config: &LocalWorkspaceConfig,
    path: &Path,
    class: &str,
) -> Result<(), LocalWorkspaceError> {
    ensure_workspace_path(config, path, class)?;
    if !fs::symlink_metadata(path)
        .map_err(io_error)?
        .file_type()
        .is_dir()
    {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "workspace cleanup target is not a directory",
        )));
    }
    let marker = fs::read_to_string(path.join(".hephaestus-workspace")).map_err(io_error)?;
    let owner = marker.trim();
    Uuid::parse_str(owner).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("workspace ownership marker is not an ID"))
    })?;
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if file_name != owner && !file_name.starts_with(&format!("{owner}.")) {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "workspace ownership marker is invalid",
        )));
    }
    make_tree_removable(path)?;
    let parent = path.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("workspace cleanup parent is missing"))
    })?;
    fs::remove_dir_all(path).map_err(io_error)?;
    sync_directory(parent)
}

fn make_tree_removable(path: &Path) -> Result<(), LocalWorkspaceError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        if fs::symlink_metadata(&child)
            .map_err(io_error)?
            .file_type()
            .is_dir()
        {
            make_tree_removable(&child)?;
        }
    }
    Ok(())
}

fn write_owner_marker(root: &Path, owner: &str) -> Result<(), LocalWorkspaceError> {
    Uuid::parse_str(owner).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("workspace owner is not an opaque ID"))
    })?;
    write_new_file(&root.join(".hephaestus-workspace"), owner.as_bytes(), false)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), LocalWorkspaceError> {
    fs::create_dir(destination).map_err(io_error)?;
    for entry in fs::read_dir(source).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.file_type().is_file() {
            fs::copy(&source_path, &destination_path).map_err(io_error)?;
            fs::set_permissions(
                &destination_path,
                fs::Permissions::from_mode(if metadata.permissions().mode() & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                }),
            )
            .map_err(io_error)?;
        } else if metadata.file_type().is_symlink() {
            symlink(
                fs::read_link(&source_path).map_err(io_error)?,
                &destination_path,
            )
            .map_err(io_error)?;
        } else {
            return Err(LocalWorkspaceError::InvalidSource(String::from(
                "source materialization contains an unsupported object",
            )));
        }
    }
    Ok(())
}

fn make_source_read_only(path: &Path) -> Result<(), LocalWorkspaceError> {
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            make_source_read_only(&child)?;
        } else if metadata.file_type().is_file() {
            let executable = metadata.permissions().mode() & 0o111 != 0;
            fs::set_permissions(
                &child,
                fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
            )
            .map_err(io_error)?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o555)).map_err(io_error)
}

fn fsync_tree(path: &Path) -> Result<(), LocalWorkspaceError> {
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            fsync_tree(&child)?;
        } else if metadata.file_type().is_file() {
            File::open(&child)
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
        }
    }
    sync_directory(path)
}

fn sync_directory(path: &Path) -> Result<(), LocalWorkspaceError> {
    File::open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
}

fn write_new_file(path: &Path, bytes: &[u8], executable: bool) -> Result<(), LocalWorkspaceError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
    )
    .map_err(io_error)
}

fn utf8_path(path: &Path) -> Result<String, LocalWorkspaceError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("path is not UTF-8")))
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    value
}

#[allow(clippy::needless_pass_by_value)] // `map_err` supplies ownership at the persistence boundary.
fn repository(error: WorkspaceRepositoryError) -> LocalWorkspaceError {
    LocalWorkspaceError::Persistence(error.to_string())
}

const fn serialization(error: serde_json::Error) -> LocalWorkspaceError {
    LocalWorkspaceError::Serialization(error)
}

const fn io_error(error: io::Error) -> LocalWorkspaceError {
    LocalWorkspaceError::Io(error)
}

const fn join_error(error: tokio::task::JoinError) -> LocalWorkspaceError {
    LocalWorkspaceError::Task(error)
}

const fn integer_error(error: std::num::TryFromIntError) -> LocalWorkspaceError {
    LocalWorkspaceError::Integer(error)
}

/// Local workspace and result publication failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LocalWorkspaceError {
    /// Static configuration is unsafe or incomplete.
    #[error("invalid workspace configuration: {0}")]
    Configuration(String),
    /// Stored or requested lifecycle state is inconsistent.
    #[error("invalid workspace state: {0}")]
    State(String),
    /// A path failed canonical layout validation.
    #[error("unsafe workspace path: {0}")]
    UnsafePath(String),
    /// A repository source tree cannot be safely materialized.
    #[error("invalid repository source: {0}")]
    InvalidSource(String),
    /// A sealed result tree cannot be safely imported.
    #[error("invalid agent result: {0}")]
    InvalidResult(String),
    /// Workspace resource limits were exceeded.
    #[error("workspace quota exceeded: {0}")]
    Quota(String),
    /// A durable or Git publication invariant was violated.
    #[error("workspace integrity failure: {0}")]
    Integrity(String),
    /// Trusted Git plumbing failed.
    #[error("trusted Git plumbing failed: {0}")]
    Git(String),
    /// Provider persistence failed.
    #[error("workspace persistence operation failed: {0}")]
    Persistence(String),
    /// Filesystem persistence failed.
    #[error("workspace filesystem operation failed: {0}")]
    Io(#[source] io::Error),
    /// Structured metadata serialization failed.
    #[error("workspace serialization failed: {0}")]
    Serialization(#[source] serde_json::Error),
    /// Blocking workspace task failed to join.
    #[error("workspace task failed: {0}")]
    Task(#[source] tokio::task::JoinError),
    /// A platform-sized value could not be persisted.
    #[error("workspace integer conversion failed: {0}")]
    Integer(#[source] std::num::TryFromIntError),
}

#[cfg(test)]
mod tests {
    use super::{
        LocalWorkspaceError, declared_regular_file, validate_message, validate_relative_path,
        validate_symlink_target,
    };
    use std::{fs, os::unix::fs::symlink};

    #[test]
    fn rejects_repository_and_symlink_path_escapes() {
        for path in [
            "../escape",
            "/absolute",
            ".git/config",
            "nested/../../escape",
        ] {
            assert!(matches!(
                validate_relative_path(path),
                Err(LocalWorkspaceError::UnsafePath(_))
            ));
        }
        for target in ["../escape", "/absolute", "nested/../escape", "."] {
            assert!(matches!(
                validate_symlink_target(target),
                Err(LocalWorkspaceError::UnsafePath(_))
            ));
        }
        validate_relative_path("reports/result.json").expect("safe repository path");
        validate_symlink_target("reports/result.json").expect("safe symlink target");
    }

    #[test]
    fn validates_result_message_bounds() {
        assert_eq!(
            validate_message("   ").expect("default result message"),
            "Hephaestus agent result"
        );
        assert!(matches!(
            validate_message(&"x".repeat(4097)),
            Err(LocalWorkspaceError::InvalidResult(_))
        ));
        assert!(matches!(
            validate_message("bad\0message"),
            Err(LocalWorkspaceError::InvalidResult(_))
        ));
    }

    #[test]
    fn declared_file_lookup_never_traverses_a_symlink() {
        let temporary = tempfile::tempdir().expect("temporary declared file");
        let work = temporary.path().join("work");
        fs::create_dir(&work).expect("work directory");
        let actual = work.join("actual");
        fs::create_dir(&actual).expect("actual directory");
        fs::write(actual.join("result.txt"), b"result").expect("actual result");
        symlink("actual", work.join("linked")).expect("linked directory");
        assert!(matches!(
            declared_regular_file(&work, "linked/result.txt"),
            Err(LocalWorkspaceError::InvalidResult(_))
        ));
        declared_regular_file(&work, "actual/result.txt").expect("direct regular file");
    }
}
