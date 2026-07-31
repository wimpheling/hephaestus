//! Isolated build execution from an exact Git commit into the one-way
//! immutable release-artifact importer.
//!
//! The build guest receives a read-only source tree and one empty writable
//! output tree. It receives no canonical Git directory, release-store path,
//! instance state volume, secret mount, or host credential.

use agent_config::{BuildArtifactKind, BuildConfig, NetworkProfile};
use release_artifact_store::{ImportedArtifact, LocalArtifactStore};
use release_domain::{
    ArtifactKind, BuildRequestId, ReleaseAgentId, ReleaseArtifactId, ReleaseCommandKey, ReleaseId,
    ReleaseVersion,
};
use release_postgres::ReleaseService;
use release_service::{CompleteBuild, ReleaseArtifactInput};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    ffi::OsStr,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::Arc,
    time::Duration,
};
use tokio::sync::broadcast;
use uuid::Uuid;
use vm_trait::{
    GuestCommand, LogStream, NetworkMode, RootFilesystem, StopMode, VmEvent, VmExit, VmId, VmMount,
    VmProvider, VmResources, VmSpec,
};

mod repository;
pub use repository::{
    BuildInput, BuildRepository, BuildRepositoryError, ClaimedBuild, FinalizationBuild,
    RecoverableBuild,
};

const SOURCE_GUEST_PATH: &str = "/workspace/source";
const OUTPUT_GUEST_PATH: &str = "/workspace/output";
const MAX_SOURCE_ENTRIES: usize = 100_000;
const MAX_SOURCE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_BUILD_LOG_BYTES: usize = 4 * 1024 * 1024;
const MAX_BUILD_METRICS: usize = 4_096;

/// Host paths, root images, and execution bounds for isolated builds.
#[derive(Debug, Clone)]
pub struct BuildExecutorConfig {
    /// Private transient build workspace root.
    pub workspace_root: PathBuf,
    /// Bare canonical repository root. Only the host materializer reads it.
    pub repository_root: PathBuf,
    /// Absolute trusted Git executable.
    pub git_binary: PathBuf,
    /// Platform-approved digest-pinned build root images.
    pub root_images: BTreeMap<String, RootFilesystem>,
    /// Maximum wall-clock duration for one build guest.
    pub timeout: Duration,
}

/// Stable result of one imported draft-producing build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildExecutionResult {
    /// Exact durable build request.
    pub build_request_id: BuildRequestId,
    /// Stable draft release created from the imported manifest.
    pub release_id: ReleaseId,
    /// Exported release agent.
    pub release_agent_id: ReleaseAgentId,
    /// Deterministic build-derived draft version.
    pub release_version: ReleaseVersion,
    /// Number of safely imported regular artifacts.
    pub artifact_count: usize,
}

/// PostgreSQL-coordinated isolated build worker.
pub struct BuildExecutor {
    repository: Arc<dyn BuildRepository>,
    provider: Arc<dyn VmProvider>,
    artifacts: LocalArtifactStore,
    releases: Arc<ReleaseService>,
    config: BuildExecutorConfig,
}

impl BuildExecutor {
    /// Validates and creates private transient roots.
    ///
    /// # Errors
    ///
    /// Rejects relative, overlapping, unsafe, or missing trusted paths.
    pub fn initialize(
        repository: Arc<dyn BuildRepository>,
        provider: Arc<dyn VmProvider>,
        artifacts: LocalArtifactStore,
        releases: Arc<ReleaseService>,
        mut config: BuildExecutorConfig,
    ) -> Result<Self, BuildExecutionError> {
        if !config.workspace_root.is_absolute()
            || !config.repository_root.is_absolute()
            || !config.git_binary.is_absolute()
            || config.timeout.is_zero()
        {
            return Err(BuildExecutionError::InvalidConfiguration);
        }
        fs::create_dir_all(config.workspace_root.join("active")).map_err(filesystem)?;
        config.workspace_root = fs::canonicalize(config.workspace_root).map_err(filesystem)?;
        config.repository_root = fs::canonicalize(config.repository_root).map_err(filesystem)?;
        config.git_binary = fs::canonicalize(config.git_binary).map_err(filesystem)?;
        if config.workspace_root.starts_with(&config.repository_root)
            || config.repository_root.starts_with(&config.workspace_root)
            || !config.git_binary.is_file()
        {
            return Err(BuildExecutionError::InvalidConfiguration);
        }
        validate_private_directory(&config.workspace_root)?;
        Ok(Self {
            repository,
            provider,
            artifacts,
            releases,
            config,
        })
    }

    /// Executes, seals, imports, and creates the immutable draft for one build.
    ///
    /// # Errors
    ///
    /// Returns a stable failure class after recording a durable failed build.
    /// Redelivery never launches a second VM for a nonterminal execution.
    #[allow(clippy::too_many_lines)]
    pub async fn execute(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        if let Some(result) = self.completed(build_request_id).await? {
            return Ok(result);
        }
        if let Some(result) = self.resume_finalization(build_request_id).await? {
            return Ok(result);
        }
        let claimed = self.claim(build_request_id).await?;
        let workspace = self.active_path(build_request_id);
        let input = claimed.input.clone();
        let materializer = self.config.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            prepare_workspace(&materializer, &input, &workspace)
        })
        .await
        .map_err(|_| BuildExecutionError::WorkerJoin)?
        .inspect_err(|_| {
            tracing::warn!(%build_request_id, "exact build source materialization failed");
        });
        let workspace = match prepared {
            Ok(workspace) => workspace,
            Err(error) => {
                self.fail(build_request_id, "source_materialization", &[], &[])
                    .await?;
                return Err(error);
            }
        };
        let spec = match self.vm_spec(&claimed, &workspace) {
            Ok(spec) => spec,
            Err(error) => {
                self.fail(build_request_id, "invalid_build_contract", &[], &[])
                    .await?;
                cleanup_workspace(&workspace.root)?;
                return Err(error);
            }
        };
        let Ok(instance) = self.provider.provision(spec).await else {
            self.fail(build_request_id, "vm_provision", &[], &[])
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        };
        let mut events = instance.subscribe_events();
        if instance.start().await.is_err() {
            drop(instance.destroy().await);
            self.fail(build_request_id, "vm_start", &[], &[]).await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        }
        self.mark_running(build_request_id).await?;
        let (exit, logs, metrics, timed_out) =
            collect_execution(&instance, &mut events, self.config.timeout).await;
        if timed_out {
            drop(instance.stop(StopMode::Force).await);
        }
        if instance.destroy().await.is_err() {
            self.fail(build_request_id, "vm_destroy", &logs, &metrics)
                .await?;
            return Err(BuildExecutionError::VmCleanup);
        }
        let exit = exit.ok_or(BuildExecutionError::Vm)?;
        if timed_out || exit.code != Some(0) || exit.signal.is_some() {
            self.fail_with_exit(build_request_id, "guest_failed", &exit, &logs, &metrics)
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::GuestFailed);
        }
        seal_output(&workspace.output)?;
        self.mark_sealed(build_request_id, &exit, &logs, &metrics)
            .await?;
        let imported = self
            .artifacts
            .import_for(build_request_id.as_uuid(), &workspace.output)?;
        validate_declared_outputs(&claimed.input.build, &imported)?;
        let release_inputs = release_inputs(build_request_id, &claimed.input.build, &imported)?;
        self.mark_imported(build_request_id, &release_inputs)
            .await?;
        self.finish_release(claimed, release_inputs).await
    }

    /// Reaps build VMs abandoned before the one-way sealed-output boundary.
    ///
    /// The durable release and execution identities are retained. Once
    /// provider cleanup is confirmed, redelivery may safely retry the exact
    /// build without creating another identity.
    ///
    /// # Errors
    ///
    /// Returns an error unless every selected orphan is confirmed destroyed
    /// and its private transient workspace is removed.
    pub async fn recover_after_restart(&self) -> Result<usize, BuildExecutionError> {
        let rows = self.repository.recoverable().await?;
        for row in &rows {
            self.provider
                .cleanup_orphan(&VmId(row.vm_id.clone()))
                .await
                .map_err(|_| BuildExecutionError::VmCleanup)?;
            cleanup_workspace(&self.active_path(row.id))?;
            self.repository.reset_after_cleanup(row.id).await?;
        }
        let finalizing = self.repository.finalizing().await?;
        for id in &finalizing {
            self.execute(*id).await?;
        }
        Ok(rows.len() + finalizing.len())
    }

    fn active_path(&self, id: BuildRequestId) -> PathBuf {
        self.config
            .workspace_root
            .join("active")
            .join(id.to_string())
    }

    async fn completed(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<BuildExecutionResult>, BuildExecutionError> {
        self.repository
            .completed(id)
            .await?
            .map(|(release_id, release_agent_id, version, artifact_count)| {
                Ok(BuildExecutionResult {
                    build_request_id: id,
                    release_id,
                    release_agent_id,
                    release_version: version,
                    artifact_count,
                })
            })
            .transpose()
    }

    async fn resume_finalization(
        &self,
        id: BuildRequestId,
    ) -> Result<Option<BuildExecutionResult>, BuildExecutionError> {
        let Some(finalization) = self.repository.finalization(id).await? else {
            return Ok(None);
        };
        let FinalizationBuild {
            state,
            claimed,
            artifact_manifest,
        } = finalization;
        let artifacts = if state == "sealed" {
            let output = self.active_path(id).join("output");
            let imported = self.artifacts.import_for(id.as_uuid(), &output)?;
            validate_declared_outputs(&claimed.input.build, &imported)?;
            let artifacts = release_inputs(id, &claimed.input.build, &imported)?;
            self.mark_imported(id, &artifacts).await?;
            artifacts
        } else {
            stored_release_inputs(artifact_manifest.ok_or(BuildExecutionError::StoredState)?)?
        };
        self.finish_release(claimed, artifacts).await.map(Some)
    }

    async fn finish_release(
        &self,
        claimed: ClaimedBuild,
        artifacts: Vec<ReleaseArtifactInput>,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        let id = claimed.input.id;
        let artifact_count = artifacts.len();
        self.releases
            .complete_build(CompleteBuild {
                command_key: ReleaseCommandKey::derive(
                    "complete-isolated-build",
                    &[id.as_uuid().as_bytes()],
                ),
                build_request_id: id,
                release_id: claimed.release_id,
                version: claimed.release_version.clone(),
                release_agent_id: claimed.release_agent_id,
                artifacts,
            })
            .await
            .map_err(|_| BuildExecutionError::Release)?;
        self.mark_drafted(id).await?;
        cleanup_workspace(&self.active_path(id))?;
        Ok(BuildExecutionResult {
            build_request_id: id,
            release_id: claimed.release_id,
            release_agent_id: claimed.release_agent_id,
            release_version: claimed.release_version,
            artifact_count,
        })
    }

    async fn claim(&self, id: BuildRequestId) -> Result<ClaimedBuild, BuildExecutionError> {
        self.repository.claim(id).await.map_err(Into::into)
    }

    fn vm_spec(
        &self,
        claimed: &ClaimedBuild,
        workspace: &PreparedBuildWorkspace,
    ) -> Result<VmSpec, BuildExecutionError> {
        let build = &claimed.input.build;
        let root = self
            .config
            .root_images
            .get(&build.root_image)
            .cloned()
            .ok_or(BuildExecutionError::RootImageDenied)?;
        let network = match build.network.profile {
            NetworkProfile::Disabled => NetworkMode::Disabled,
            NetworkProfile::Egress => NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            NetworkProfile::BrokerOnly => return Err(BuildExecutionError::NetworkDenied),
        };
        Ok(VmSpec {
            id: VmId(format!("build-{}", claimed.input.id)),
            root,
            disks: Vec::new(),
            mounts: vec![
                VmMount {
                    tag: String::from("build-source"),
                    host_path: workspace.source.clone(),
                    guest_path: PathBuf::from(SOURCE_GUEST_PATH),
                    read_only: true,
                },
                VmMount {
                    tag: String::from("build-output"),
                    host_path: workspace.output.clone(),
                    guest_path: PathBuf::from(OUTPUT_GUEST_PATH),
                    read_only: false,
                },
            ],
            resources: VmResources {
                vcpus: build.resources.vcpus,
                memory_mib: build.resources.memory_mib,
            },
            network,
            command: GuestCommand {
                program: build.command.clone(),
                args: build.arguments.clone(),
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from(&build.working_directory)),
            },
            labels: BTreeMap::from([
                (String::from("hephaestus.kind"), String::from("build")),
                (
                    String::from("hephaestus.build-request-id"),
                    claimed.input.id.to_string(),
                ),
                (
                    String::from("hephaestus.source-ref"),
                    claimed.input.source_ref.clone(),
                ),
            ]),
        })
    }

    async fn mark_running(&self, id: BuildRequestId) -> Result<(), BuildExecutionError> {
        self.repository.mark_running(id).await.map_err(Into::into)
    }

    async fn mark_sealed(
        &self,
        id: BuildRequestId,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .mark_sealed(id, exit, logs, metrics)
            .await
            .map_err(Into::into)
    }

    async fn mark_imported(
        &self,
        id: BuildRequestId,
        artifacts: &[ReleaseArtifactInput],
    ) -> Result<(), BuildExecutionError> {
        let manifest = artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact.id,
                    "path": artifact.path,
                    "kind": artifact_kind(artifact.kind),
                    "mode": artifact.mode,
                    "content_hash": artifact.content_hash,
                    "size_bytes": artifact.size_bytes,
                    "media_type": artifact.media_type,
                    "storage_key": artifact.storage_key,
                })
            })
            .collect::<Vec<_>>();
        self.repository
            .mark_imported(id, &manifest)
            .await
            .map_err(Into::into)
    }

    async fn mark_drafted(&self, id: BuildRequestId) -> Result<(), BuildExecutionError> {
        self.repository.mark_drafted(id).await.map_err(Into::into)
    }

    async fn fail(
        &self,
        id: BuildRequestId,
        code: &str,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .fail(id, code, None, None, logs, metrics)
            .await
            .map_err(Into::into)
    }

    async fn fail_with_exit(
        &self,
        id: BuildRequestId,
        code: &str,
        exit: &VmExit,
        logs: &[Value],
        metrics: &[Value],
    ) -> Result<(), BuildExecutionError> {
        self.repository
            .fail(id, code, exit.code, exit.signal, logs, metrics)
            .await
            .map_err(Into::into)
    }
}

struct PreparedBuildWorkspace {
    root: PathBuf,
    source: PathBuf,
    output: PathBuf,
}

fn prepare_workspace(
    config: &BuildExecutorConfig,
    input: &BuildInput,
    active: &Path,
) -> Result<PreparedBuildWorkspace, BuildExecutionError> {
    if active.exists() {
        return Err(BuildExecutionError::UnsafeWorkspace);
    }
    let repository = config
        .repository_root
        .join(format!("{}.git", input.repository_id));
    let repository = fs::canonicalize(repository).map_err(filesystem)?;
    if repository.parent() != Some(config.repository_root.as_path()) || !repository.is_dir() {
        return Err(BuildExecutionError::UnsafeRepository);
    }
    let staging = config
        .workspace_root
        .join(format!(".prepare-{}", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(filesystem)?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    let source = staging.join("source");
    let output = staging.join("output");
    fs::create_dir(&source).map_err(filesystem)?;
    fs::create_dir(&output).map_err(filesystem)?;
    if let Err(error) = materialize_source(config, &repository, &input.source_commit, &source) {
        drop(fs::remove_dir_all(&staging));
        return Err(error);
    }
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    fs::rename(&staging, active).map_err(filesystem)?;
    Ok(PreparedBuildWorkspace {
        root: active.to_path_buf(),
        source: active.join("source"),
        output: active.join("output"),
    })
}

fn materialize_source(
    config: &BuildExecutorConfig,
    repository: &Path,
    commit: &str,
    source: &Path,
) -> Result<(), BuildExecutionError> {
    let tree = git_output(
        config,
        repository,
        &["ls-tree", "-rz", "-r", "--full-tree", commit],
    )?;
    let records = tree
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    if records.len() > MAX_SOURCE_ENTRIES {
        return Err(BuildExecutionError::SourceQuota);
    }
    let mut total = 0_u64;
    for record in records {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(BuildExecutionError::InvalidGitTree)?;
        let metadata =
            std::str::from_utf8(&record[..tab]).map_err(|_| BuildExecutionError::InvalidGitTree)?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| BuildExecutionError::InvalidGitTree)?;
        validate_relative_path(path)?;
        let mut fields = metadata.split_ascii_whitespace();
        let mode =
            u32::from_str_radix(fields.next().ok_or(BuildExecutionError::InvalidGitTree)?, 8)
                .map_err(|_| BuildExecutionError::InvalidGitTree)?;
        if fields.next() != Some("blob") || !matches!(mode, 0o100_644 | 0o100_755) {
            return Err(BuildExecutionError::UnsupportedSourceObject);
        }
        let object = fields.next().ok_or(BuildExecutionError::InvalidGitTree)?;
        let bytes = git_output(config, repository, &["cat-file", "blob", object])?;
        total = total
            .checked_add(u64::try_from(bytes.len()).map_err(|_| BuildExecutionError::SourceQuota)?)
            .ok_or(BuildExecutionError::SourceQuota)?;
        if total > MAX_SOURCE_BYTES {
            return Err(BuildExecutionError::SourceQuota);
        }
        let destination = source.join(path);
        fs::create_dir_all(
            destination
                .parent()
                .ok_or(BuildExecutionError::InvalidGitTree)?,
        )
        .map_err(filesystem)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if mode == 0o100_755 { 0o500 } else { 0o400 })
            .open(destination)
            .map_err(filesystem)?;
        file.write_all(&bytes).map_err(filesystem)?;
        file.flush().map_err(filesystem)?;
    }
    seal_source_directories(source)
}

fn seal_source_directories(path: &Path) -> Result<(), BuildExecutionError> {
    for entry in fs::read_dir(path).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            seal_source_directories(&entry.path())?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o500)).map_err(filesystem)
}

fn git_output(
    config: &BuildExecutorConfig,
    repository: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, BuildExecutionError> {
    let output = Command::new(&config.git_binary)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("--git-dir")
        .arg(repository)
        .args(arguments)
        .output()
        .map_err(filesystem)?;
    if !output.status.success() {
        return Err(BuildExecutionError::Git);
    }
    Ok(output.stdout)
}

fn validate_relative_path(value: &str) -> Result<(), BuildExecutionError> {
    if value.is_empty() || value.len() > 1_024 || value.contains('\\') {
        return Err(BuildExecutionError::InvalidGitTree);
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || matches!(component, Component::Normal(name) if name == OsStr::new(".git"))
        })
    {
        return Err(BuildExecutionError::InvalidGitTree);
    }
    Ok(())
}

async fn collect_execution(
    instance: &Arc<dyn vm_trait::VmInstance>,
    events: &mut broadcast::Receiver<VmEvent>,
    timeout: Duration,
) -> (Option<VmExit>, Vec<Value>, Vec<Value>, bool) {
    let wait = instance.wait();
    tokio::pin!(wait);
    let deadline = tokio::time::sleep(timeout);
    tokio::pin!(deadline);
    let mut logs = Vec::new();
    let mut metrics = Vec::new();
    let mut log_bytes = 0_usize;
    loop {
        tokio::select! {
            result = &mut wait => {
                drain_execution_events(events, &mut logs, &mut metrics, &mut log_bytes);
                return (result.ok(), logs, metrics, false);
            },
            () = &mut deadline => return (None, logs, metrics, true),
            event = events.recv() => match event {
                Ok(event) => capture_execution_event(
                    event,
                    &mut logs,
                    &mut metrics,
                    &mut log_bytes,
                ),
                Err(
                    broadcast::error::RecvError::Lagged(_)
                    | broadcast::error::RecvError::Closed,
                ) => {}
            }
        }
    }
}

fn drain_execution_events(
    events: &mut broadcast::Receiver<VmEvent>,
    logs: &mut Vec<Value>,
    metrics: &mut Vec<Value>,
    log_bytes: &mut usize,
) {
    loop {
        match events.try_recv() {
            Ok(event) => capture_execution_event(event, logs, metrics, log_bytes),
            Err(broadcast::error::TryRecvError::Lagged(_)) => {}
            Err(broadcast::error::TryRecvError::Empty | broadcast::error::TryRecvError::Closed) => {
                return;
            }
        }
    }
}

fn capture_execution_event(
    event: VmEvent,
    logs: &mut Vec<Value>,
    metrics: &mut Vec<Value>,
    log_bytes: &mut usize,
) {
    match event {
        VmEvent::Log { stream, bytes } => {
            if let Some(next) = log_bytes.checked_add(bytes.len())
                && next <= MAX_BUILD_LOG_BYTES
            {
                *log_bytes = next;
                logs.push(json!({
                    "stream": match stream {
                        LogStream::Stdout => "stdout",
                        LogStream::Stderr => "stderr",
                        _ => "unknown",
                    },
                    "text": String::from_utf8_lossy(&bytes),
                }));
            }
        }
        VmEvent::Metric(metric) if metrics.len() < MAX_BUILD_METRICS => {
            metrics.push(json!({
                "name": metric.name,
                "value": metric.value,
                "labels": metric.labels,
            }));
        }
        _ => {}
    }
}

fn seal_output(root: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(BuildExecutionError::UnsafeOutput);
    }
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            seal_output(&entry.path())?;
        } else if metadata.file_type().is_file() && metadata.nlink() == 1 {
            let executable = metadata.permissions().mode() & 0o111 != 0;
            fs::set_permissions(
                entry.path(),
                fs::Permissions::from_mode(if executable { 0o500 } else { 0o400 }),
            )
            .map_err(filesystem)?;
        } else {
            return Err(BuildExecutionError::UnsafeOutput);
        }
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o500)).map_err(filesystem)
}

fn validate_declared_outputs(
    build: &BuildConfig,
    imported: &[ImportedArtifact],
) -> Result<(), BuildExecutionError> {
    for artifact in imported {
        if !build.artifacts.iter().any(|declared| {
            let path = artifact.path.as_str();
            match declared.kind {
                BuildArtifactKind::Directory => path
                    .strip_prefix(&declared.path)
                    .is_some_and(|suffix| suffix.starts_with('/')),
                BuildArtifactKind::File => {
                    path == declared.path && artifact.kind == ArtifactKind::File
                }
                BuildArtifactKind::Executable => {
                    path == declared.path && artifact.kind == ArtifactKind::Executable
                }
            }
        }) {
            return Err(BuildExecutionError::UndeclaredOutput);
        }
    }
    for declared in &build.artifacts {
        let present = imported.iter().any(|artifact| match declared.kind {
            BuildArtifactKind::Directory => artifact
                .path
                .as_str()
                .strip_prefix(&declared.path)
                .is_some_and(|suffix| suffix.starts_with('/')),
            BuildArtifactKind::File | BuildArtifactKind::Executable => {
                artifact.path.as_str() == declared.path
            }
        });
        if !present {
            return Err(BuildExecutionError::MissingOutput);
        }
    }
    Ok(())
}

fn release_inputs(
    build_request_id: BuildRequestId,
    build: &BuildConfig,
    imported: &[ImportedArtifact],
) -> Result<Vec<ReleaseArtifactInput>, BuildExecutionError> {
    imported
        .iter()
        .map(|artifact| {
            let declaration = build
                .artifacts
                .iter()
                .find(|declared| {
                    artifact.path.as_str() == declared.path
                        || (declared.kind == BuildArtifactKind::Directory
                            && artifact
                                .path
                                .as_str()
                                .strip_prefix(&declared.path)
                                .is_some_and(|suffix| suffix.starts_with('/')))
                })
                .ok_or(BuildExecutionError::UndeclaredOutput)?;
            Ok(ReleaseArtifactInput {
                id: stable_artifact_id(build_request_id, artifact.path.as_str()),
                path: artifact.path.clone(),
                kind: artifact.kind,
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                media_type: declaration
                    .media_type
                    .clone()
                    .unwrap_or_else(|| String::from("application/octet-stream")),
                storage_key: artifact.storage_key,
            })
        })
        .collect()
}

fn stable_artifact_id(build_request_id: BuildRequestId, path: &str) -> ReleaseArtifactId {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.release-artifact-id.v1");
    digest.update(build_request_id.as_uuid().as_bytes());
    digest.update((path.len() as u64).to_be_bytes());
    digest.update(path.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ReleaseArtifactId::from_uuid(Uuid::from_bytes(bytes))
}

#[derive(Deserialize)]
struct StoredArtifact {
    id: ReleaseArtifactId,
    path: release_domain::ArtifactPath,
    kind: ArtifactKind,
    mode: u16,
    content_hash: release_domain::ContentHash,
    size_bytes: u64,
    media_type: String,
    storage_key: Uuid,
}

fn stored_release_inputs(value: Value) -> Result<Vec<ReleaseArtifactInput>, BuildExecutionError> {
    let stored: Vec<StoredArtifact> =
        serde_json::from_value(value).map_err(|_| BuildExecutionError::StoredState)?;
    if stored.is_empty() {
        return Err(BuildExecutionError::StoredState);
    }
    Ok(stored
        .into_iter()
        .map(|artifact| ReleaseArtifactInput {
            id: artifact.id,
            path: artifact.path,
            kind: artifact.kind,
            mode: artifact.mode,
            content_hash: artifact.content_hash,
            size_bytes: artifact.size_bytes,
            media_type: artifact.media_type,
            storage_key: artifact.storage_key,
        })
        .collect())
}

const fn artifact_kind(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Executable => "executable",
        ArtifactKind::File => "file",
        ArtifactKind::Manifest => "manifest",
        ArtifactKind::BuildLog => "build_log",
    }
}

fn cleanup_workspace(root: &Path) -> Result<(), BuildExecutionError> {
    if !root.exists() {
        return Ok(());
    }
    make_removable(root)?;
    fs::remove_dir_all(root).map_err(filesystem)
}

fn make_removable(root: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(BuildExecutionError::UnsafeWorkspace);
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            make_removable(&entry.path())?;
        } else if !metadata.file_type().is_file() {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
    }
    Ok(())
}

fn validate_private_directory(path: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(BuildExecutionError::InvalidConfiguration);
    }
    Ok(())
}

fn filesystem(_error: std::io::Error) -> BuildExecutionError {
    BuildExecutionError::Filesystem
}

/// Stable non-sensitive build execution failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildExecutionError {
    /// Static host configuration is unsafe.
    #[error("isolated build configuration is invalid")]
    InvalidConfiguration,
    /// Build request or exact configuration is unavailable.
    #[error("isolated build request is unavailable")]
    Unavailable,
    /// A durable execution already owns this build.
    #[error("isolated build request is already claimed")]
    AlreadyClaimed,
    /// The original requester no longer has permission to execute this build.
    #[error("isolated build execution is not authorized")]
    Unauthorized,
    /// The authorization provider could not make a safe decision.
    #[error("isolated build authorization failed")]
    Authorization,
    /// Stored build state is malformed.
    #[error("isolated build durable state is invalid")]
    StoredState,
    /// Build source repository path is unsafe.
    #[error("isolated build repository is unsafe")]
    UnsafeRepository,
    /// Transient workspace path is unsafe.
    #[error("isolated build workspace is unsafe")]
    UnsafeWorkspace,
    /// Source tree exceeds a platform bound.
    #[error("isolated build source exceeds a platform bound")]
    SourceQuota,
    /// Git tree encoding or path is invalid.
    #[error("isolated build source tree is invalid")]
    InvalidGitTree,
    /// Source contains an unsupported object such as a symlink or submodule.
    #[error("isolated build source contains an unsupported object")]
    UnsupportedSourceObject,
    /// Trusted Git inspection failed.
    #[error("isolated build Git inspection failed")]
    Git,
    /// Selected build root image is not platform approved.
    #[error("isolated build root image is denied")]
    RootImageDenied,
    /// Selected build network policy is not supported.
    #[error("isolated build network policy is denied")]
    NetworkDenied,
    /// VM provisioning, start, or monitoring failed.
    #[error("isolated build VM operation failed")]
    Vm,
    /// VM cleanup could not be confirmed, so import was not attempted.
    #[error("isolated build VM cleanup is incomplete")]
    VmCleanup,
    /// Guest exited unsuccessfully or timed out.
    #[error("isolated build guest failed")]
    GuestFailed,
    /// Sealed output includes an unsafe object.
    #[error("isolated build output is unsafe")]
    UnsafeOutput,
    /// Guest emitted a path not declared by the immutable build contract.
    #[error("isolated build produced an undeclared output")]
    UndeclaredOutput,
    /// A required declared output was absent.
    #[error("isolated build omitted a declared output")]
    MissingOutput,
    /// Safe one-way artifact import failed.
    #[error(transparent)]
    Artifact(#[from] release_artifact_store::ArtifactStoreError),
    /// Draft release construction failed after import.
    #[error("isolated build release finalization failed")]
    Release,
    /// Durable persistence failed.
    #[error(transparent)]
    Database(#[from] BuildRepositoryError),
    /// A validated release value could not be reconstructed.
    #[error(transparent)]
    ReleaseValue(#[from] release_domain::ReleaseValueError),
    /// A blocking worker failed to join.
    #[error("isolated build worker failed")]
    WorkerJoin,
    /// Host filesystem operation failed.
    #[error("isolated build filesystem operation failed")]
    Filesystem,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const CONFIG: &str = r#"
version = 2
[agent]
name = "builder"
key = "builder"
[build]
command = "/usr/bin/build"
working_directory = "/workspace/source"
root_image = "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 128
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/agent"
kind = "executable"
[guest]
command = "bin/agent"
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "run@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
refs = []
"#;

    #[test]
    fn materializes_two_exact_commits_without_git_metadata() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let repositories = temporary.path().join("repositories");
        let workspaces = temporary.path().join("builds");
        let source_repository = temporary.path().join("source");
        fs::create_dir(&repositories).expect("repository root");
        fs::create_dir_all(workspaces.join("active")).expect("workspace root");
        fs::create_dir(&source_repository).expect("source repository");
        git(&source_repository, &["init", "--initial-branch=main"]);
        git(&source_repository, &["config", "user.name", "Build Test"]);
        git(
            &source_repository,
            &["config", "user.email", "build@example.invalid"],
        );
        fs::write(source_repository.join("input.txt"), b"first\n").expect("first source");
        git(&source_repository, &["add", "input.txt"]);
        git(&source_repository, &["commit", "-m", "first"]);
        let first = git_text(&source_repository, &["rev-parse", "HEAD"]);
        fs::write(source_repository.join("input.txt"), b"second\n").expect("second source");
        git(&source_repository, &["commit", "-am", "second"]);
        let second = git_text(&source_repository, &["rev-parse", "HEAD"]);
        let repository_id = Uuid::new_v4();
        git(
            temporary.path(),
            &[
                "clone",
                "--bare",
                source_repository.to_str().expect("source path"),
                repositories
                    .join(format!("{repository_id}.git"))
                    .to_str()
                    .expect("bare path"),
            ],
        );
        let config = BuildExecutorConfig {
            workspace_root: fs::canonicalize(&workspaces).expect("workspace path"),
            repository_root: fs::canonicalize(&repositories).expect("repository path"),
            git_binary: fs::canonicalize("/usr/bin/git").expect("Git binary"),
            root_images: BTreeMap::new(),
            timeout: Duration::from_secs(30),
        };
        let build = agent_config::parse(CONFIG.as_bytes())
            .config
            .expect("valid config")
            .build
            .expect("build config");
        for (index, (commit, expected)) in [
            (first, b"first\n".as_slice()),
            (second, b"second\n".as_slice()),
        ]
        .into_iter()
        .enumerate()
        {
            let id = BuildRequestId::new();
            let input = BuildInput {
                id,
                repository_id,
                source_commit: commit,
                source_ref: String::from("refs/heads/main"),
                build: build.clone(),
            };
            let active = config
                .workspace_root
                .join("active")
                .join(format!("{id}-{index}"));
            let prepared =
                prepare_workspace(&config, &input, &active).expect("materialize exact commit");
            assert_eq!(
                fs::read(prepared.source.join("input.txt")).expect("materialized source"),
                expected
            );
            assert!(!prepared.source.join(".git").exists());
            assert_eq!(
                fs::metadata(&prepared.source)
                    .expect("source metadata")
                    .permissions()
                    .mode()
                    & 0o222,
                0
            );
            assert!(
                fs::read_dir(&prepared.output)
                    .expect("empty output")
                    .next()
                    .is_none()
            );
            cleanup_workspace(&prepared.root).expect("cleanup workspace");
        }
    }

    #[test]
    fn sealing_rejects_guest_symlinks() {
        let temporary = tempfile::tempdir().expect("temporary root");
        let output = temporary.path().join("output");
        fs::create_dir(&output).expect("output");
        symlink("/etc/passwd", output.join("escape")).expect("guest symlink");
        assert!(matches!(
            seal_output(&output),
            Err(BuildExecutionError::UnsafeOutput)
        ));
    }

    fn git(directory: &Path, arguments: &[&str]) {
        let output = Command::new("/usr/bin/git")
            .args(arguments)
            .current_dir(directory)
            .output()
            .expect("run Git");
        assert!(
            output.status.success(),
            "Git failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn git_text(directory: &Path, arguments: &[&str]) -> String {
        let output = Command::new("/usr/bin/git")
            .args(arguments)
            .current_dir(directory)
            .output()
            .expect("run Git");
        assert!(output.status.success());
        String::from_utf8(output.stdout)
            .expect("Git UTF-8")
            .trim()
            .to_owned()
    }
}
