//! Local exact-runtime materialization for immutable release artifacts and
//! host-generated run context.
//!
//! Repository paths are used only inside a fresh administrator-owned staging
//! tree. Canonical artifact storage is addressed exclusively by opaque UUID,
//! and every object is rehashed while copied into the non-reusable run tree.

use async_trait::async_trait;
use release_domain::ArtifactPath;
use run_domain::{Run, RunKind};
use run_orchestrator::{
    MailboxRuntimeEvent, PreparedRunRuntime, RunRuntimeArtifact, RunRuntimeArtifactKind,
    RunRuntimeCatalog, RunRuntimeCatalogError, RunRuntimeError, RunRuntimeInput, RunRuntimeManager,
};
use runtime_types::RunId;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
};
use uuid::Uuid;
use vm_trait::VmMount;

const MAX_RUNTIME_ARTIFACTS: usize = 4_096;
const MAX_RUNTIME_BYTES: u64 = 512 * 1024 * 1024;
const RELEASE_TAG_PREFIX: &str = "rel";
const CONTEXT_TAG_PREFIX: &str = "ctx";
const PREVIOUS_RELEASE_TAG_PREFIX: &str = "old";
const GATEWAY_RELEASE_TAG_PREFIX: &str = "gwr";

/// Filesystem roots used for per-run runtime materialization.
#[derive(Debug, Clone)]
pub struct LocalRunRuntimeConfig {
    /// Transient administrator-owned root containing active run trees.
    pub runtime_root: PathBuf,
    /// Durable administrator-owned opaque release-object store.
    pub release_artifact_root: PathBuf,
}

/// Local exact-runtime lifecycle manager over a provider-neutral catalog.
#[derive(Clone)]
pub struct LocalRunRuntimeManager {
    catalog: Arc<dyn RunRuntimeCatalog>,
    config: LocalRunRuntimeConfig,
}

/// Local immutable release tree used by a one-shot gateway invocation.
///
/// Gateway invocations share the same verified object store and artifact
/// materialization rules as runs, but have no run-shaped control context.
/// Keeping this narrow adapter here prevents gateway dispatch from mounting
/// opaque object-store files directly as a release filesystem.
#[derive(Clone)]
pub struct LocalGatewayReleaseRuntime {
    runtime_root: PathBuf,
    release_artifact_root: PathBuf,
}

impl LocalRunRuntimeManager {
    /// Validates, creates, and canonicalizes the configured roots.
    ///
    /// # Errors
    ///
    /// Rejects relative, overlapping, symlink, non-directory, or
    /// group/world-writable roots.
    pub fn initialize(
        catalog: Arc<dyn RunRuntimeCatalog>,
        mut config: LocalRunRuntimeConfig,
    ) -> Result<Self, RunRuntimeError> {
        if !config.runtime_root.is_absolute() || !config.release_artifact_root.is_absolute() {
            return Err(runtime_error("runtime roots must be absolute"));
        }
        fs::create_dir_all(config.runtime_root.join("active")).map_err(filesystem)?;
        fs::create_dir_all(&config.release_artifact_root).map_err(filesystem)?;
        config.runtime_root = fs::canonicalize(config.runtime_root).map_err(filesystem)?;
        config.release_artifact_root =
            fs::canonicalize(config.release_artifact_root).map_err(filesystem)?;
        if config
            .runtime_root
            .starts_with(&config.release_artifact_root)
            || config
                .release_artifact_root
                .starts_with(&config.runtime_root)
        {
            return Err(runtime_error("runtime and release roots must not overlap"));
        }
        validate_root(&config.runtime_root)?;
        validate_root(&config.release_artifact_root)?;
        Ok(Self { catalog, config })
    }

    fn active_path(&self, run_id: RunId) -> PathBuf {
        self.config
            .runtime_root
            .join("active")
            .join(run_id.to_string())
    }

    /// Returns a gateway-specific release materializer over these validated
    /// runtime roots.
    #[must_use]
    pub fn gateway_release_runtime(&self) -> LocalGatewayReleaseRuntime {
        LocalGatewayReleaseRuntime {
            runtime_root: self.config.runtime_root.clone(),
            release_artifact_root: self.config.release_artifact_root.clone(),
        }
    }

    async fn load(&self, run: &Run) -> Result<RunRuntimeInput, RunRuntimeError> {
        let input = self.catalog.load_runtime(run).await.map_err(catalog)?;
        if input.artifacts.is_empty() || input.artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        if run.kind == RunKind::Update && input.previous_artifacts.is_empty() {
            return Err(runtime_error("previous release artifacts are unavailable"));
        }
        if input.previous_artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("previous release artifact count is invalid"));
        }
        Ok(input)
    }

    fn materialize(
        &self,
        run: &Run,
        input: &RunRuntimeInput,
    ) -> Result<PreparedRunRuntime, RunRuntimeError> {
        let staging = self
            .config
            .runtime_root
            .join(format!(".prepare-{}", Uuid::new_v4()));
        create_directory(&staging, 0o700)?;
        let result = self.materialize_staging(run, input, &staging);
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    // Staging the complete runtime is deliberately kept in one linear routine so
    // every validation failure shares the caller's atomic cleanup path.
    #[allow(clippy::cognitive_complexity)]
    fn materialize_staging(
        &self,
        run: &Run,
        input: &RunRuntimeInput,
        staging: &Path,
    ) -> Result<PreparedRunRuntime, RunRuntimeError> {
        let release = staging.join("release");
        let previous_release = staging.join("release-previous");
        let control = staging.join("control");
        create_directory(&release, 0o700)?;
        create_directory(&control, 0o700)?;
        let mut total = 0_u64;
        for artifact in &input.artifacts {
            total = total
                .checked_add(artifact.size_bytes)
                .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
            if total > MAX_RUNTIME_BYTES {
                return Err(runtime_error("release artifact size is invalid"));
            }
            materialize_artifact(&self.config.release_artifact_root, &release, artifact)?;
        }
        tracing::debug!(run_id = %run.id, "release artifacts materialized");
        if !input.previous_artifacts.is_empty() {
            create_directory(&previous_release, 0o700)?;
            for artifact in &input.previous_artifacts {
                total = total
                    .checked_add(artifact.size_bytes)
                    .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
                if total > MAX_RUNTIME_BYTES {
                    return Err(runtime_error("release artifact size is invalid"));
                }
                materialize_artifact(
                    &self.config.release_artifact_root,
                    &previous_release,
                    artifact,
                )?;
            }
            make_tree_read_only(&previous_release)?;
        }
        write_json(&control.join("parameters.json"), &input.parameters)?;
        tracing::debug!(run_id = %run.id, "runtime parameters materialized");
        if let Some(mailbox_event) = input.mailbox_event.as_ref() {
            materialize_mailbox_event(&control, mailbox_event)?;
        }
        let context = HostContext::new(run, input);
        write_json(&control.join("context.json"), &context)?;
        tracing::debug!(run_id = %run.id, "runtime context materialized");
        if let Some(parameters) = input.previous_parameters.as_ref() {
            write_json(&control.join("parameters-previous.json"), parameters)?;
        }
        make_tree_read_only(&release)?;
        tracing::debug!(run_id = %run.id, "release tree sealed");
        make_tree_read_only(&control)?;
        tracing::debug!(run_id = %run.id, "control tree sealed");
        let active = self.active_path(run.id);
        fs::rename(staging, &active).map_err(filesystem)?;
        fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
        tracing::debug!(run_id = %run.id, "runtime root activated and sealed");
        let mut mounts = vec![
            VmMount {
                tag: runtime_mount_tag(RELEASE_TAG_PREFIX, run.id),
                host_path: active.join("release"),
                guest_path: PathBuf::from("/release"),
                read_only: true,
            },
            VmMount {
                tag: runtime_mount_tag(CONTEXT_TAG_PREFIX, run.id),
                host_path: active.join("control"),
                guest_path: PathBuf::from("/run/hephaestus"),
                read_only: true,
            },
        ];
        if !input.previous_artifacts.is_empty() {
            mounts.push(VmMount {
                tag: runtime_mount_tag(PREVIOUS_RELEASE_TAG_PREFIX, run.id),
                host_path: active.join("release-previous"),
                guest_path: PathBuf::from("/release-previous"),
                read_only: true,
            });
        }
        Ok(PreparedRunRuntime { mounts })
    }
}

impl LocalGatewayReleaseRuntime {
    fn active_path(&self, invocation_id: Uuid) -> PathBuf {
        self.runtime_root
            .join("gateways")
            .join(invocation_id.to_string())
    }

    /// Materializes verified artifacts and exact revision parameters into
    /// read-only `/release` and `/run/hephaestus` trees.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when artifact metadata, the canonical store,
    /// or the local runtime tree is invalid.
    pub fn prepare(
        &self,
        invocation_id: Uuid,
        artifacts: &[RunRuntimeArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, RunRuntimeError> {
        if artifacts.is_empty() || artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        let parameter_bytes = serde_json::to_vec(parameters)
            .map_err(|_| runtime_error("gateway parameters are invalid"))?;
        if !parameters.is_object() || parameter_bytes.len() > 65_536 {
            return Err(runtime_error("gateway parameters exceed the object bound"));
        }
        let gateways = self.runtime_root.join("gateways");
        fs::create_dir_all(&gateways).map_err(filesystem)?;
        fs::set_permissions(&gateways, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
        let active = self.active_path(invocation_id);
        if active.exists() {
            return Err(runtime_error("gateway runtime already exists"));
        }
        let staging = self
            .runtime_root
            .join(format!(".gateway-prepare-{invocation_id}"));
        create_directory(&staging, 0o700)?;
        let release = staging.join("release");
        let result = (|| {
            create_directory(&release, 0o700)?;
            let mut total = 0_u64;
            for artifact in artifacts {
                total = total
                    .checked_add(artifact.size_bytes)
                    .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
                if total > MAX_RUNTIME_BYTES {
                    return Err(runtime_error("release artifact size is invalid"));
                }
                materialize_artifact(&self.release_artifact_root, &release, artifact)?;
            }
            make_tree_read_only(&release)?;
            let control = staging.join("control");
            create_directory(&control, 0o700)?;
            write_bytes(&control.join("parameters.json"), &parameter_bytes)?;
            make_tree_read_only(&control)?;
            fs::rename(&staging, &active).map_err(filesystem)?;
            fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
            Ok::<_, RunRuntimeError>(vec![
                VmMount {
                    tag: gateway_runtime_mount_tag(invocation_id),
                    host_path: active.join("release"),
                    guest_path: PathBuf::from("/release"),
                    read_only: true,
                },
                VmMount {
                    tag: format!("gwp-{}", invocation_id.simple()),
                    host_path: active.join("control"),
                    guest_path: PathBuf::from("/run/hephaestus"),
                    read_only: true,
                },
            ])
        })();
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Removes one gateway release tree after its VM has been destroyed.
    ///
    /// # Errors
    ///
    /// Returns a redacted error when the path is not a safe local runtime tree.
    pub fn destroy(&self, invocation_id: Uuid) -> Result<(), RunRuntimeError> {
        let active = self.active_path(invocation_id);
        match fs::symlink_metadata(&active) {
            Ok(_) => {
                make_tree_removable(&active)?;
                fs::remove_dir_all(active).map_err(filesystem)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }
}

#[async_trait]
impl RunRuntimeManager for LocalRunRuntimeManager {
    async fn prepare(&self, run: &Run) -> Result<PreparedRunRuntime, RunRuntimeError> {
        if self.active_path(run.id).exists() {
            return Err(runtime_error("run runtime already exists"));
        }
        let input = self.load(run).await?;
        self.materialize(run, &input)
    }

    async fn destroy(&self, run_id: RunId) -> Result<(), RunRuntimeError> {
        let path = self.active_path(run_id);
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
                make_tree_removable(&path)?;
                fs::remove_dir_all(path).map_err(filesystem)
            }
            Ok(_) => Err(runtime_error("run runtime root is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }

    async fn recover(&self) -> Result<usize, RunRuntimeError> {
        let active = self.config.runtime_root.join("active");
        let mut recovered = 0;
        for entry in fs::read_dir(active).map_err(filesystem)? {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("run runtime entry is invalid"))?;
            let uuid = Uuid::parse_str(&name)
                .map_err(|_| runtime_error("run runtime entry is invalid"))?;
            let live = self
                .catalog
                .run_is_live(RunId::from_uuid(uuid))
                .await
                .map_err(catalog)?;
            if !live {
                self.destroy(RunId::from_uuid(uuid)).await?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
}

#[derive(Serialize)]
struct HostContext<'a> {
    schema_version: u8,
    run_id: RunId,
    run_kind: RunKind,
    instance_id: runtime_types::AgentInstanceId,
    instance_revision_id: runtime_types::AgentInstanceRevisionId,
    release_id: runtime_types::ReleaseId,
    release_agent_id: runtime_types::ReleaseAgentId,
    attachment_id: Option<runtime_types::AgentAttachmentId>,
    repository_id: Option<Uuid>,
    git_ref: Option<&'a str>,
    commit_sha: Option<&'a str>,
    release_mount: &'static str,
    repository_mount: &'static str,
    work_mount: &'static str,
    state_mount: Option<&'static str>,
    parameters_path: &'static str,
    update_id: Option<Uuid>,
    previous_revision_id: Option<Uuid>,
    previous_release_id: Option<Uuid>,
    previous_release_mount: Option<&'static str>,
    previous_parameters_path: Option<&'static str>,
    mailbox_event_path: Option<&'static str>,
    mailbox_body_path: Option<&'static str>,
}

impl<'a> HostContext<'a> {
    fn new(run: &Run, input: &'a RunRuntimeInput) -> Self {
        Self {
            schema_version: 1,
            run_id: run.id,
            run_kind: run.kind,
            instance_id: run.instance_id,
            instance_revision_id: run.instance_revision_id,
            release_id: run.release_id,
            release_agent_id: run.release_agent_id,
            attachment_id: run.attachment_id,
            repository_id: input.repository_id,
            git_ref: input.git_ref.as_deref(),
            commit_sha: input.commit_sha.as_deref(),
            release_mount: "/release",
            repository_mount: "/workspace/repo",
            work_mount: "/workspace/work",
            state_mount: run.requires_state.then_some("/var/lib/hephaestus"),
            parameters_path: "/run/hephaestus/parameters.json",
            update_id: input.update_id,
            previous_revision_id: input.previous_revision_id,
            previous_release_id: input.previous_release_id,
            previous_release_mount: input.previous_release_id.map(|_| "/release-previous"),
            previous_parameters_path: input
                .previous_parameters
                .as_ref()
                .map(|_| "/run/hephaestus/parameters-previous.json"),
            mailbox_event_path: input
                .mailbox_event
                .as_ref()
                .map(|_| "/run/hephaestus/mailbox-event.json"),
            mailbox_body_path: input
                .mailbox_event
                .as_ref()
                .map(|_| "/run/hephaestus/mailbox-body"),
        }
    }
}

#[derive(Serialize)]
struct MailboxControlEnvelope<'a> {
    schema_version: u8,
    mailbox_id: Uuid,
    event_id: Uuid,
    body_id: Uuid,
    method: &'a str,
    route: &'a str,
    selected_headers: &'a serde_json::Value,
    content_type: Option<&'a str>,
    trace_context: Option<&'a str>,
    received_at: time::OffsetDateTime,
    body_path: &'static str,
}

fn materialize_mailbox_event(
    control: &Path,
    event: &MailboxRuntimeEvent,
) -> Result<(), RunRuntimeError> {
    if event.body.len() > 1_048_576 {
        return Err(runtime_error("mailbox event body is invalid"));
    }
    if Sha256::digest(&event.body).as_slice() != event.integrity_hash {
        return Err(runtime_error("mailbox event body integrity is invalid"));
    }
    let envelope = MailboxControlEnvelope {
        schema_version: 1,
        mailbox_id: event.mailbox_id,
        event_id: event.event_id,
        body_id: event.body_id,
        method: &event.method,
        route: &event.route,
        selected_headers: &event.selected_headers,
        content_type: event.content_type.as_deref(),
        trace_context: event.trace_context.as_deref(),
        received_at: event.received_at,
        body_path: "/run/hephaestus/mailbox-body",
    };
    write_json(&control.join("mailbox-event.json"), &envelope)?;
    write_bytes(&control.join("mailbox-body"), &event.body)
}

fn materialize_artifact(
    store_root: &Path,
    release_root: &Path,
    artifact: &RunRuntimeArtifact,
) -> Result<(), RunRuntimeError> {
    let relative = ArtifactPath::parse(artifact.path.clone())
        .map_err(|_| runtime_error("release artifact path is invalid"))?;
    let expected_mode = match artifact.kind {
        RunRuntimeArtifactKind::Executable => 0o555,
        RunRuntimeArtifactKind::File | RunRuntimeArtifactKind::Manifest => 0o444,
    };
    if artifact.mode != expected_mode {
        return Err(runtime_error("release artifact metadata is invalid"));
    }
    let source = store_root.join(artifact.storage_key.simple().to_string());
    let destination = release_root.join(relative.as_str());
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).map_err(filesystem)?;
    }
    let mut input = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow())
        .open(source)
        .map_err(filesystem)?;
    let metadata = input.metadata().map_err(filesystem)?;
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.len() != artifact.size_bytes
    {
        return Err(runtime_error("canonical release object is invalid"));
    }
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(&destination)
        .map_err(filesystem)?;
    let mut digest = Sha256::new();
    let mut length = 0_u64;
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = input.read(&mut buffer).map_err(filesystem)?;
        if count == 0 {
            break;
        }
        length = length
            .checked_add(
                u64::try_from(count)
                    .map_err(|_| runtime_error("release artifact size is invalid"))?,
            )
            .ok_or_else(|| runtime_error("release artifact size is invalid"))?;
        digest.update(&buffer[..count]);
        output.write_all(&buffer[..count]).map_err(filesystem)?;
    }
    output.flush().map_err(filesystem)?;
    if length != artifact.size_bytes || <[u8; 32]>::from(digest.finalize()) != artifact.content_hash
    {
        return Err(runtime_error(
            "canonical release object failed verification",
        ));
    }
    fs::set_permissions(destination, fs::Permissions::from_mode(expected_mode)).map_err(filesystem)
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), RunRuntimeError> {
    let bytes = serde_json::to_vec(value).map_err(serialization)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(path)
        .map_err(filesystem)?;
    file.write_all(&bytes).map_err(filesystem)?;
    file.write_all(b"\n").map_err(filesystem)?;
    file.flush().map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(filesystem)
}

fn write_bytes(path: &Path, bytes: &[u8]) -> Result<(), RunRuntimeError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o400)
        .custom_flags(o_nofollow())
        .open(path)
        .map_err(filesystem)?;
    file.write_all(bytes).map_err(filesystem)?;
    file.flush().map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o444)).map_err(filesystem)
}

fn make_tree_read_only(root: &Path) -> Result<(), RunRuntimeError> {
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = entry.metadata().map_err(filesystem)?;
        if metadata.file_type().is_dir() {
            make_tree_read_only(&entry.path())?;
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(0o555))
                .map_err(filesystem)?;
        }
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o555)).map_err(filesystem)
}

fn make_tree_removable(root: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(runtime_error("run runtime tree is unsafe"));
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            make_tree_removable(&entry.path())?;
        } else if !metadata.file_type().is_file() {
            return Err(runtime_error("run runtime tree is unsafe"));
        }
    }
    Ok(())
}

fn create_directory(path: &Path, mode: u32) -> Result<(), RunRuntimeError> {
    fs::create_dir(path).map_err(filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(filesystem)
}

fn validate_root(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(runtime_error("runtime root is unsafe"));
    }
    Ok(())
}

fn runtime_mount_tag(prefix: &str, run_id: RunId) -> String {
    format!("{prefix}-{}", run_id.as_uuid().simple())
}

fn gateway_runtime_mount_tag(invocation_id: Uuid) -> String {
    format!("{GATEWAY_RELEASE_TAG_PREFIX}-{}", invocation_id.simple())
}

#[cfg(target_os = "linux")]
const fn o_nofollow() -> i32 {
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
const fn o_nofollow() -> i32 {
    0
}

fn runtime_error(message: impl Into<String>) -> RunRuntimeError {
    RunRuntimeError::redacted(message)
}

// Error details may contain host paths or catalog diagnostics, so only stable classes
// cross the runtime-manager boundary.
#[allow(clippy::needless_pass_by_value)]
fn filesystem(error: std::io::Error) -> RunRuntimeError {
    tracing::warn!(
        error_kind = ?error.kind(),
        raw_os_error = ?error.raw_os_error(),
        "run runtime filesystem operation failed"
    );
    runtime_error("runtime filesystem operation failed")
}

// See `filesystem`: catalog diagnostics are intentionally redacted.
#[allow(clippy::needless_pass_by_value)]
fn catalog(_error: RunRuntimeCatalogError) -> RunRuntimeError {
    runtime_error("runtime provenance query failed")
}

// See `filesystem`: serialized context values must not enter diagnostics.
#[allow(clippy::needless_pass_by_value)]
fn serialization(_error: serde_json::Error) -> RunRuntimeError {
    runtime_error("runtime context serialization failed")
}

#[cfg(test)]
mod tests {
    use super::{
        CONTEXT_TAG_PREFIX, LocalGatewayReleaseRuntime, PREVIOUS_RELEASE_TAG_PREFIX,
        RELEASE_TAG_PREFIX, make_tree_read_only, materialize_artifact, materialize_mailbox_event,
        runtime_mount_tag,
    };
    use release_artifact_store::LocalArtifactStore;
    use run_orchestrator::{MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeArtifactKind};
    use runtime_types::RunId;
    use sha2::{Digest, Sha256};
    use std::{fs, os::unix::fs::PermissionsExt, process::Command};
    use uuid::Uuid;

    #[test]
    fn mailbox_input_is_sealed_as_a_control_envelope_and_opaque_body() {
        let fixture = tempfile::tempdir().expect("fixture");
        let mailbox_id = Uuid::new_v4();
        let event_id = Uuid::new_v4();
        let body_id = Uuid::new_v4();
        let body = b"guest-only-mailbox-body".to_vec();
        materialize_mailbox_event(
            fixture.path(),
            &MailboxRuntimeEvent {
                mailbox_id,
                event_id,
                body_id,
                method: String::from("POST"),
                route: String::from("/event"),
                selected_headers: serde_json::json!({"x-kind": "proof"}),
                content_type: Some(String::from("application/octet-stream")),
                trace_context: None,
                received_at: time::OffsetDateTime::now_utc(),
                body: body.clone(),
                integrity_hash: Sha256::digest(&body).into(),
            },
        )
        .expect("materialize mailbox input");
        assert_eq!(
            fs::read(fixture.path().join("mailbox-body")).expect("read body"),
            body
        );
        let envelope: serde_json::Value = serde_json::from_slice(
            &fs::read(fixture.path().join("mailbox-event.json")).expect("read envelope"),
        )
        .expect("parse envelope");
        assert_eq!(envelope["mailbox_id"], mailbox_id.to_string());
        assert_eq!(envelope["event_id"], event_id.to_string());
        assert_eq!(envelope["body_id"], body_id.to_string());
        assert_eq!(envelope["body_path"], "/run/hephaestus/mailbox-body");
    }

    #[test]
    fn runtime_mount_tags_fit_the_libkrun_limit() {
        let run_id = RunId::new();

        for prefix in [
            RELEASE_TAG_PREFIX,
            CONTEXT_TAG_PREFIX,
            PREVIOUS_RELEASE_TAG_PREFIX,
        ] {
            let tag = runtime_mount_tag(prefix, run_id);
            assert_eq!(tag.len(), 36);
            assert_eq!(tag, runtime_mount_tag(prefix, run_id));
        }
    }

    #[test]
    fn gateway_runtime_materializes_and_removes_the_exact_release_tree() {
        let fixture = tempfile::tempdir().expect("fixture");
        let runtime_root = fixture.path().join("runtime");
        let store_root = fixture.path().join("store");
        fs::create_dir(&runtime_root).expect("runtime root");
        fs::create_dir(&store_root).expect("store root");
        let key = Uuid::new_v4();
        let bytes = b"gateway release executable";
        fs::write(store_root.join(key.simple().to_string()), bytes).expect("object");
        let invocation = Uuid::new_v4();
        let runtime = LocalGatewayReleaseRuntime {
            runtime_root: runtime_root.clone(),
            release_artifact_root: store_root,
        };

        let mounts = runtime
            .prepare(
                invocation,
                &[RunRuntimeArtifact {
                    path: String::from("bin/handler"),
                    kind: RunRuntimeArtifactKind::Executable,
                    mode: 0o555,
                    content_hash: Sha256::digest(bytes).into(),
                    size_bytes: u64::try_from(bytes.len()).expect("length"),
                    storage_key: key,
                }],
                &serde_json::json!({"inbound_placeholder":"public-identifier","alice_provider_id":1001}),
            )
            .expect("materialize gateway release");
        let mount = &mounts[0];
        assert_eq!(mounts.len(), 2);
        let control = &mounts[1];
        assert_eq!(
            control.guest_path,
            std::path::PathBuf::from("/run/hephaestus")
        );
        assert!(control.read_only);
        let parameters: serde_json::Value = serde_json::from_slice(
            &fs::read(control.host_path.join("parameters.json")).expect("sealed parameters"),
        )
        .expect("parameter JSON");
        assert_eq!(parameters["alice_provider_id"], 1001);
        assert_eq!(parameters["inbound_placeholder"], "public-identifier");
        assert_eq!(
            fs::metadata(control.host_path.join("parameters.json"))
                .expect("parameter mode")
                .permissions()
                .mode()
                & 0o222,
            0,
            "ordinary parameters are sealed before guest launch"
        );
        assert_eq!(mount.guest_path, std::path::PathBuf::from("/release"));
        assert!(mount.read_only);
        assert_eq!(
            fs::read(mount.host_path.join("bin/handler")).expect("materialized executable"),
            bytes
        );

        runtime
            .destroy(invocation)
            .expect("destroy gateway release");
        assert!(
            !runtime_root
                .join("gateways")
                .join(invocation.to_string())
                .exists()
        );
    }

    #[test]
    fn materializes_verified_artifact_with_declared_mode() {
        let fixture = tempfile::tempdir().expect("fixture");
        let store = fixture.path().join("store");
        let release = fixture.path().join("release");
        fs::create_dir(&store).expect("store");
        fs::create_dir(&release).expect("release");
        let key = Uuid::new_v4();
        let bytes = b"exact release executable";
        fs::write(store.join(key.simple().to_string()), bytes).expect("object");
        let artifact = RunRuntimeArtifact {
            path: String::from("bin/agent"),
            kind: RunRuntimeArtifactKind::Executable,
            mode: 0o555,
            content_hash: Sha256::digest(bytes).into(),
            size_bytes: u64::try_from(bytes.len()).expect("length"),
            storage_key: key,
        };

        materialize_artifact(&store, &release, &artifact).expect("materialize");

        let output = release.join("bin/agent");
        assert_eq!(fs::read(&output).expect("output"), bytes);
        assert_eq!(
            fs::metadata(output).expect("metadata").permissions().mode() & 0o777,
            0o555
        );
    }

    #[test]
    fn rejects_tampered_canonical_object() {
        let fixture = tempfile::tempdir().expect("fixture");
        let store = fixture.path().join("store");
        let release = fixture.path().join("release");
        fs::create_dir(&store).expect("store");
        fs::create_dir(&release).expect("release");
        let key = Uuid::new_v4();
        fs::write(store.join(key.simple().to_string()), b"tampered").expect("object");
        let artifact = RunRuntimeArtifact {
            path: String::from("agent"),
            kind: RunRuntimeArtifactKind::File,
            mode: 0o444,
            content_hash: [0; 32],
            size_bytes: 8,
            storage_key: key,
        };

        assert!(materialize_artifact(&store, &release, &artifact).is_err());
    }

    #[test]
    fn executes_only_the_imported_read_only_release_artifact() {
        let fixture = tempfile::tempdir().expect("fixture");
        let store_root = fixture.path().join("store");
        let sealed_output = fixture.path().join("sealed-output");
        let source_tree = fixture.path().join("source");
        let release_tree = fixture.path().join("release");
        fs::create_dir(&store_root).expect("store");
        fs::set_permissions(&store_root, fs::Permissions::from_mode(0o700)).expect("store mode");
        fs::create_dir_all(sealed_output.join("bin")).expect("sealed output");
        fs::create_dir_all(source_tree.join("bin")).expect("source tree");
        fs::create_dir(&release_tree).expect("release tree");

        let built = sealed_output.join("bin/agent");
        fs::write(&built, b"#!/bin/sh\nprintf 'imported-release\\n'\n").expect("built executable");
        fs::set_permissions(&built, fs::Permissions::from_mode(0o755)).expect("built mode");
        let source_decoy = source_tree.join("bin/agent");
        fs::write(&source_decoy, b"#!/bin/sh\nexit 97\n").expect("source decoy");
        fs::set_permissions(&source_decoy, fs::Permissions::from_mode(0o755)).expect("source mode");

        let store = LocalArtifactStore::new(store_root.clone()).expect("artifact store");
        let imported = store
            .import_for(Uuid::new_v4(), &sealed_output)
            .expect("safe one-way import");
        assert_eq!(imported.len(), 1);
        let artifact = &imported[0];
        materialize_artifact(
            &store_root,
            &release_tree,
            &RunRuntimeArtifact {
                path: artifact.path.as_str().to_owned(),
                kind: RunRuntimeArtifactKind::Executable,
                mode: u32::from(artifact.mode),
                content_hash: *artifact.content_hash.as_bytes(),
                size_bytes: artifact.size_bytes,
                storage_key: artifact.storage_key,
            },
        )
        .expect("verified runtime materialization");
        make_tree_read_only(&release_tree).expect("seal release tree");

        let executable = release_tree.join("bin/agent");
        let output = Command::new(&executable)
            .current_dir(&source_tree)
            .output()
            .expect("execute imported release artifact");
        assert!(output.status.success());
        assert_eq!(output.stdout, b"imported-release\n");
        assert_eq!(
            fs::metadata(&executable)
                .expect("executable metadata")
                .permissions()
                .mode()
                & 0o777,
            0o555
        );
        assert_eq!(
            fs::metadata(&release_tree)
                .expect("release metadata")
                .permissions()
                .mode()
                & 0o777,
            0o555
        );
    }
}
