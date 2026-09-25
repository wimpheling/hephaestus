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
use serde::{Deserialize, Serialize};
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
const GATEWAY_SERVICE_NAMESPACE: &str = "gateway-services";
const GATEWAY_SERVICE_STAGING_PREFIX: &str = ".prepare-";
const GATEWAY_SERVICE_METADATA: &str = "identity.json";
const GATEWAY_SERVICE_SCHEMA_VERSION: u8 = 1;
const MAX_GATEWAY_SERVICE_METADATA_BYTES: u64 = 1_024;

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

/// Immutable identity of one host-owned persistent gateway service instance.
///
/// A new launch attempt receives a new `instance_id`; the gateway and revision
/// IDs remain sealed in the materialized tree for recovery and cleanup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayServiceIdentity {
    /// Unique host-owned launch-attempt identity.
    pub instance_id: Uuid,
    /// Durable gateway aggregate identity.
    pub gateway_id: Uuid,
    /// Immutable gateway revision selected for this instance.
    pub revision_id: Uuid,
}

/// One validated persistent-service tree discovered during recovery.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GatewayServiceInstanceRecord {
    /// Sealed identity read from the tree metadata.
    pub identity: GatewayServiceIdentity,
    /// Host path below the dedicated service namespace.
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GatewayServiceIdentityFile {
    schema_version: u8,
    instance_id: Uuid,
    gateway_id: Uuid,
    revision_id: Uuid,
}

impl From<GatewayServiceIdentity> for GatewayServiceIdentityFile {
    fn from(identity: GatewayServiceIdentity) -> Self {
        Self {
            schema_version: GATEWAY_SERVICE_SCHEMA_VERSION,
            instance_id: identity.instance_id,
            gateway_id: identity.gateway_id,
            revision_id: identity.revision_id,
        }
    }
}

impl TryFrom<GatewayServiceIdentityFile> for GatewayServiceIdentity {
    type Error = RunRuntimeError;

    fn try_from(metadata: GatewayServiceIdentityFile) -> Result<Self, Self::Error> {
        if metadata.schema_version != GATEWAY_SERVICE_SCHEMA_VERSION
            || metadata.instance_id.is_nil()
            || metadata.gateway_id.is_nil()
            || metadata.revision_id.is_nil()
        {
            return Err(runtime_error(
                "gateway service identity metadata is invalid",
            ));
        }
        Ok(Self {
            instance_id: metadata.instance_id,
            gateway_id: metadata.gateway_id,
            revision_id: metadata.revision_id,
        })
    }
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

    fn service_namespace(&self) -> PathBuf {
        self.runtime_root.join(GATEWAY_SERVICE_NAMESPACE)
    }

    fn service_path(&self, instance_id: Uuid) -> PathBuf {
        self.service_namespace().join(instance_id.to_string())
    }

    fn ensure_service_namespace(&self) -> Result<PathBuf, RunRuntimeError> {
        ensure_existing_directory(&self.runtime_root)?;
        let namespace = self.service_namespace();
        match fs::symlink_metadata(&namespace) {
            Ok(metadata)
                if metadata.file_type().is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o022 == 0 =>
            {
                Ok(namespace)
            }
            Ok(_) => Err(runtime_error("gateway service namespace is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                create_directory(&namespace, 0o700)?;
                Ok(namespace)
            }
            Err(error) => Err(filesystem(error)),
        }
    }

    fn existing_service_namespace(&self) -> Result<Option<PathBuf>, RunRuntimeError> {
        ensure_existing_directory(&self.runtime_root)?;
        let namespace = self.service_namespace();
        match fs::symlink_metadata(&namespace) {
            Ok(metadata)
                if metadata.file_type().is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.permissions().mode() & 0o022 == 0 =>
            {
                Ok(Some(namespace))
            }
            Ok(_) => Err(runtime_error("gateway service namespace is unsafe")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(filesystem(error)),
        }
    }

    fn validate_service_identity(identity: GatewayServiceIdentity) -> Result<(), RunRuntimeError> {
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
        {
            return Err(runtime_error("gateway service identity is invalid"));
        }
        Ok(())
    }

    /// Materializes one persistent service instance in the dedicated
    /// `gateway-services` namespace. Existing instance paths fail closed;
    /// retries must use a fresh instance identity after explicit cleanup.
    ///
    /// The identity metadata is host-only and is not included in the guest
    /// mounts. Release and control trees are sealed before activation.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the identity, artifact set,
    /// namespace, staging tree, or sealed materialization is invalid.
    pub fn prepare_service(
        &self,
        identity: GatewayServiceIdentity,
        artifacts: &[RunRuntimeArtifact],
        parameters: &serde_json::Value,
    ) -> Result<Vec<VmMount>, RunRuntimeError> {
        Self::validate_service_identity(identity)?;
        if artifacts.is_empty() || artifacts.len() > MAX_RUNTIME_ARTIFACTS {
            return Err(runtime_error("release artifact count is invalid"));
        }
        let parameter_bytes = serde_json::to_vec(parameters)
            .map_err(|_| runtime_error("gateway parameters are invalid"))?;
        if !parameters.is_object() || parameter_bytes.len() > 65_536 {
            return Err(runtime_error("gateway parameters exceed the object bound"));
        }
        let namespace = self.ensure_service_namespace()?;
        let active = namespace.join(identity.instance_id.to_string());
        match fs::symlink_metadata(&active) {
            Ok(_) => return Err(runtime_error("gateway service instance already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        let staging = namespace.join(format!(
            "{GATEWAY_SERVICE_STAGING_PREFIX}{}",
            identity.instance_id
        ));
        match fs::symlink_metadata(&staging) {
            Ok(_) => return Err(runtime_error("gateway service staging already exists")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(filesystem(error)),
        }
        create_directory(&staging, 0o700)?;
        let release = staging.join("release");
        let control = staging.join("control");
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
            create_directory(&control, 0o700)?;
            write_bytes(&control.join("parameters.json"), &parameter_bytes)?;
            make_tree_read_only(&control)?;
            let metadata: GatewayServiceIdentityFile = identity.into();
            write_json(&staging.join(GATEWAY_SERVICE_METADATA), &metadata)?;
            make_tree_read_only(&staging)?;
            fs::rename(&staging, &active).map_err(filesystem)?;
            fs::set_permissions(&active, fs::Permissions::from_mode(0o500)).map_err(filesystem)?;
            Ok::<_, RunRuntimeError>(vec![
                VmMount {
                    tag: gateway_service_mount_tag("gws", identity.instance_id),
                    host_path: active.join("release"),
                    guest_path: PathBuf::from("/release"),
                    read_only: true,
                },
                VmMount {
                    tag: gateway_service_mount_tag("gwt", identity.instance_id),
                    host_path: active.join("control"),
                    guest_path: PathBuf::from("/run/hephaestus"),
                    read_only: true,
                },
            ])
        })();
        if result.is_err()
            && let Ok(metadata) = fs::symlink_metadata(&staging)
            && metadata.file_type().is_dir()
            && !metadata.file_type().is_symlink()
            && make_tree_removable(&staging).is_ok()
        {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }

    /// Enumerates validated active service trees in the dedicated namespace.
    /// Staging trees are handled separately by [`Self::cleanup_service_staging`].
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the namespace, identity metadata,
    /// or an active service tree is unsafe or malformed.
    pub fn enumerate_service_instances(
        &self,
    ) -> Result<Vec<GatewayServiceInstanceRecord>, RunRuntimeError> {
        let Some(namespace) = self.existing_service_namespace()? else {
            return Ok(Vec::new());
        };
        let entries = fs::read_dir(&namespace).map_err(filesystem)?;
        let mut records = Vec::new();
        for entry in entries {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("gateway service entry is invalid"))?;
            if name.starts_with(GATEWAY_SERVICE_STAGING_PREFIX) {
                continue;
            }
            let instance_id = Uuid::parse_str(&name)
                .map_err(|_| runtime_error("gateway service entry is invalid"))?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(filesystem)?;
            if !metadata.file_type().is_dir()
                || metadata.file_type().is_symlink()
                || metadata.permissions().mode() & 0o222 != 0
            {
                return Err(runtime_error("gateway service tree is unsafe"));
            }
            let identity = read_service_identity(&path.join(GATEWAY_SERVICE_METADATA))?;
            if identity.instance_id != instance_id {
                return Err(runtime_error(
                    "gateway service identity does not match path",
                ));
            }
            validate_sealed_service_subtree(&path.join("release"))?;
            validate_sealed_service_subtree(&path.join("control"))?;
            records.push(GatewayServiceInstanceRecord { identity, path });
        }
        records.sort_by_key(|record| record.identity.instance_id);
        Ok(records)
    }

    /// Destroys one service tree only after its exact sealed identity matches.
    /// Callers must confirm provider ownership is quiescent before invoking it.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the identity does not match or
    /// the selected tree is unsafe to remove.
    pub fn destroy_service(&self, identity: GatewayServiceIdentity) -> Result<(), RunRuntimeError> {
        Self::validate_service_identity(identity)?;
        let _namespace = self.ensure_service_namespace()?;
        let active = self.service_path(identity.instance_id);
        match fs::symlink_metadata(&active) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                    return Err(runtime_error("gateway service tree is unsafe"));
                }
                if read_service_identity(&active.join(GATEWAY_SERVICE_METADATA))? != identity {
                    return Err(runtime_error("gateway service identity mismatch"));
                }
                make_tree_removable(&active)?;
                fs::remove_dir_all(active).map_err(filesystem)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(filesystem(error)),
        }
    }

    /// Removes only stale staging trees below `gateway-services`.
    ///
    /// The service supervisor must call this only after establishing that no
    /// materialization operation is still running.
    ///
    /// # Errors
    ///
    /// Returns a redacted runtime error when the namespace or a staging tree
    /// is unsafe or malformed.
    pub fn cleanup_service_staging(&self) -> Result<usize, RunRuntimeError> {
        let Some(namespace) = self.existing_service_namespace()? else {
            return Ok(0);
        };
        let entries = fs::read_dir(&namespace).map_err(filesystem)?;
        let mut cleaned = 0;
        for entry in entries {
            let entry = entry.map_err(filesystem)?;
            let name = entry
                .file_name()
                .into_string()
                .map_err(|_| runtime_error("gateway service staging entry is invalid"))?;
            let Some(instance) = name.strip_prefix(GATEWAY_SERVICE_STAGING_PREFIX) else {
                continue;
            };
            Uuid::parse_str(instance)
                .map_err(|_| runtime_error("gateway service staging entry is invalid"))?;
            let staging = entry.path();
            make_tree_removable(&staging)?;
            fs::remove_dir_all(staging).map_err(filesystem)?;
            cleaned += 1;
        }
        Ok(cleaned)
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

fn ensure_existing_directory(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(runtime_error("runtime directory is unsafe"));
    }
    Ok(())
}

fn read_service_identity(path: &Path) -> Result<GatewayServiceIdentity, RunRuntimeError> {
    let initial = fs::symlink_metadata(path).map_err(filesystem)?;
    validate_service_identity_metadata(&initial)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(o_nofollow() | o_nonblock())
        .open(path)
        .map_err(filesystem)?;
    let metadata = file.metadata().map_err(filesystem)?;
    validate_service_identity_metadata(&metadata)?;
    let limit =
        usize::try_from(MAX_GATEWAY_SERVICE_METADATA_BYTES).expect("metadata limit fits in usize");
    let mut bytes = Vec::with_capacity(limit);
    file.take(MAX_GATEWAY_SERVICE_METADATA_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(filesystem)?;
    if bytes.len() > limit {
        return Err(runtime_error(
            "gateway service identity metadata is too large",
        ));
    }
    let metadata: GatewayServiceIdentityFile = serde_json::from_slice(&bytes)
        .map_err(|_| runtime_error("gateway service identity metadata is invalid"))?;
    metadata.try_into()
}

fn validate_service_identity_metadata(metadata: &std::fs::Metadata) -> Result<(), RunRuntimeError> {
    if !metadata.file_type().is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o222 != 0
        || metadata.len() > MAX_GATEWAY_SERVICE_METADATA_BYTES
    {
        return Err(runtime_error("gateway service identity metadata is unsafe"));
    }
    Ok(())
}

fn validate_sealed_service_subtree(path: &Path) -> Result<(), RunRuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o222 != 0
    {
        return Err(runtime_error("gateway service mount tree is unsafe"));
    }
    for entry in fs::read_dir(path).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() {
            if metadata.file_type().is_symlink() {
                return Err(runtime_error("gateway service mount tree is unsafe"));
            }
            validate_sealed_service_subtree(&entry.path())?;
        } else if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o222 != 0 {
            return Err(runtime_error("gateway service mount tree is unsafe"));
        }
    }
    Ok(())
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

fn gateway_service_mount_tag(prefix: &str, instance_id: Uuid) -> String {
    format!("{prefix}-{}", instance_id.simple())
}

#[cfg(target_os = "linux")]
const fn o_nofollow() -> i32 {
    0o400_000 | 0o2_000_000
}

#[cfg(not(target_os = "linux"))]
const fn o_nofollow() -> i32 {
    0
}

#[cfg(target_os = "linux")]
const fn o_nonblock() -> i32 {
    0o000_4000
}

#[cfg(not(target_os = "linux"))]
const fn o_nonblock() -> i32 {
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
        CONTEXT_TAG_PREFIX, GATEWAY_SERVICE_METADATA, GATEWAY_SERVICE_SCHEMA_VERSION,
        GATEWAY_SERVICE_STAGING_PREFIX, GatewayServiceIdentity, LocalGatewayReleaseRuntime,
        MAX_GATEWAY_SERVICE_METADATA_BYTES, PREVIOUS_RELEASE_TAG_PREFIX, RELEASE_TAG_PREFIX,
        make_tree_read_only, materialize_artifact, materialize_mailbox_event, runtime_mount_tag,
    };
    use release_artifact_store::LocalArtifactStore;
    use run_orchestrator::{MailboxRuntimeEvent, RunRuntimeArtifact, RunRuntimeArtifactKind};
    use runtime_types::RunId;
    use sha2::{Digest, Sha256};
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
        process::Command,
    };
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

    fn service_fixture() -> (
        tempfile::TempDir,
        LocalGatewayReleaseRuntime,
        RunRuntimeArtifact,
    ) {
        let fixture = tempfile::tempdir().expect("fixture");
        let runtime_root = fixture.path().join("runtime");
        let store_root = fixture.path().join("store");
        fs::create_dir(&runtime_root).expect("runtime root");
        fs::create_dir(&store_root).expect("store root");
        let key = Uuid::new_v4();
        let bytes = b"persistent gateway service executable";
        fs::write(store_root.join(key.simple().to_string()), bytes).expect("object");
        let runtime = LocalGatewayReleaseRuntime {
            runtime_root,
            release_artifact_root: store_root,
        };
        let artifact = RunRuntimeArtifact {
            path: String::from("bin/server"),
            kind: RunRuntimeArtifactKind::Executable,
            mode: 0o555,
            content_hash: Sha256::digest(bytes).into(),
            size_bytes: u64::try_from(bytes.len()).expect("length"),
            storage_key: key,
        };
        (fixture, runtime, artifact)
    }

    fn service_identity() -> GatewayServiceIdentity {
        GatewayServiceIdentity {
            instance_id: Uuid::new_v4(),
            gateway_id: Uuid::new_v4(),
            revision_id: Uuid::new_v4(),
        }
    }

    #[test]
    fn service_materialization_seals_identity_and_mounts_only_guest_trees() {
        let (_fixture, runtime, artifact) = service_fixture();
        let identity = service_identity();
        let mounts = runtime
            .prepare_service(identity, &[artifact], &serde_json::json!({"port": 8080}))
            .expect("materialize service");
        assert_eq!(mounts.len(), 2);
        assert!(mounts.iter().all(|mount| mount.read_only));
        assert!(mounts.iter().all(|mount| mount.tag.len() <= 36));
        assert_eq!(mounts[0].guest_path, std::path::PathBuf::from("/release"));
        assert_eq!(
            mounts[1].guest_path,
            std::path::PathBuf::from("/run/hephaestus")
        );
        assert!(
            mounts
                .iter()
                .all(|mount| !mount.host_path.ends_with(GATEWAY_SERVICE_METADATA))
        );

        let active = runtime.service_path(identity.instance_id);
        let identity_path = active.join(GATEWAY_SERVICE_METADATA);
        let metadata: serde_json::Value =
            serde_json::from_slice(&fs::read(&identity_path).expect("identity metadata"))
                .expect("identity JSON");
        assert_eq!(metadata["schema_version"], GATEWAY_SERVICE_SCHEMA_VERSION);
        assert_eq!(metadata["gateway_id"], identity.gateway_id.to_string());
        assert_eq!(
            fs::metadata(identity_path)
                .expect("identity mode")
                .permissions()
                .mode()
                & 0o222,
            0
        );
        assert_eq!(runtime.enumerate_service_instances().unwrap().len(), 1);
    }

    #[test]
    fn service_identity_rejects_writable_oversized_and_nonregular_metadata() {
        let (_fixture, runtime, artifact) = service_fixture();
        let identity = service_identity();
        runtime
            .prepare_service(identity, &[artifact], &serde_json::json!({}))
            .expect("materialize service");
        fs::set_permissions(
            runtime
                .service_path(identity.instance_id)
                .join(GATEWAY_SERVICE_METADATA),
            fs::Permissions::from_mode(0o644),
        )
        .expect("make identity writable");
        assert!(runtime.enumerate_service_instances().is_err());

        let (_fixture, runtime, artifact) = service_fixture();
        let identity = service_identity();
        runtime
            .prepare_service(identity, &[artifact], &serde_json::json!({}))
            .expect("materialize service");
        let active = runtime.service_path(identity.instance_id);
        fs::set_permissions(&active, fs::Permissions::from_mode(0o700)).expect("open active");
        let metadata_path = active.join(GATEWAY_SERVICE_METADATA);
        fs::remove_file(&metadata_path).expect("remove identity");
        fs::write(
            &metadata_path,
            vec![
                b'x';
                usize::try_from(MAX_GATEWAY_SERVICE_METADATA_BYTES)
                    .expect("metadata limit fits in usize")
                    + 1
            ],
        )
        .expect("write oversized identity");
        fs::set_permissions(&metadata_path, fs::Permissions::from_mode(0o444))
            .expect("seal oversized identity");
        assert!(runtime.enumerate_service_instances().is_err());

        let (_fixture, runtime, artifact) = service_fixture();
        let identity = service_identity();
        runtime
            .prepare_service(identity, &[artifact], &serde_json::json!({}))
            .expect("materialize service");
        let active = runtime.service_path(identity.instance_id);
        fs::set_permissions(&active, fs::Permissions::from_mode(0o700)).expect("open active");
        let metadata_path = active.join(GATEWAY_SERVICE_METADATA);
        fs::remove_file(&metadata_path).expect("remove identity");
        assert!(
            Command::new("mkfifo")
                .arg(&metadata_path)
                .status()
                .expect("mkfifo")
                .success()
        );
        assert!(runtime.enumerate_service_instances().is_err());
    }

    #[test]
    fn service_instances_coexist_and_identity_mismatch_fails_closed() {
        let (_fixture, runtime, artifact) = service_fixture();
        let first = service_identity();
        let second = service_identity();
        runtime
            .prepare_service(
                first,
                std::slice::from_ref(&artifact),
                &serde_json::json!({}),
            )
            .expect("first service");
        runtime
            .prepare_service(
                second,
                std::slice::from_ref(&artifact),
                &serde_json::json!({}),
            )
            .expect("second service");

        let records = runtime.enumerate_service_instances().unwrap();
        assert_eq!(records.len(), 2);
        let mismatch = GatewayServiceIdentity {
            gateway_id: Uuid::new_v4(),
            ..first
        };
        assert!(
            runtime
                .prepare_service(
                    mismatch,
                    std::slice::from_ref(&artifact),
                    &serde_json::json!({})
                )
                .is_err()
        );
        assert!(runtime.destroy_service(mismatch).is_err());
        assert!(runtime.service_path(first.instance_id).exists());

        runtime.destroy_service(first).expect("first cleanup");
        assert!(runtime.service_path(second.instance_id).exists());
        runtime.destroy_service(second).expect("second cleanup");
        assert!(runtime.enumerate_service_instances().unwrap().is_empty());
    }

    #[test]
    fn service_symlinks_and_malicious_paths_fail_without_cross_namespace_deletion() {
        let (_fixture, runtime, artifact) = service_fixture();
        let identity = service_identity();
        let outside = runtime.runtime_root.join("outside");
        fs::create_dir(&outside).expect("outside");
        fs::write(outside.join("keep"), b"keep").expect("outside sentinel");
        let namespace = runtime.ensure_service_namespace().expect("namespace");
        symlink(&outside, namespace.join(identity.instance_id.to_string()))
            .expect("instance symlink");
        assert!(runtime.destroy_service(identity).is_err());
        assert!(outside.join("keep").exists());
        assert!(runtime.enumerate_service_instances().is_err());

        fs::remove_file(namespace.join(identity.instance_id.to_string())).expect("symlink");
        let staging_id = Uuid::new_v4();
        let staging = namespace.join(format!("{GATEWAY_SERVICE_STAGING_PREFIX}{staging_id}"));
        symlink(&outside, &staging).expect("staging symlink");
        assert!(runtime.cleanup_service_staging().is_err());
        assert!(outside.join("keep").exists());
        fs::remove_file(&staging).expect("staging symlink");

        let broken = namespace.join(identity.instance_id.to_string());
        symlink("missing-target", &broken).expect("broken instance symlink");
        assert!(
            runtime
                .prepare_service(identity, &[artifact], &serde_json::json!({}))
                .is_err()
        );
        fs::remove_file(&broken).expect("broken instance symlink");

        fs::remove_dir(&namespace).expect("namespace");
        symlink(&outside, &namespace).expect("namespace symlink");
        assert!(runtime.enumerate_service_instances().is_err());
        assert!(runtime.cleanup_service_staging().is_err());
        assert!(outside.join("keep").exists());
    }

    #[test]
    fn failed_service_materialization_cleans_staging_and_scoped_cleanup_is_safe() {
        let (_fixture, runtime, mut artifact) = service_fixture();
        artifact.content_hash = [0; 32];
        let identity = service_identity();
        assert!(
            runtime
                .prepare_service(
                    identity,
                    std::slice::from_ref(&artifact),
                    &serde_json::json!({})
                )
                .is_err()
        );
        let namespace = runtime.service_namespace();
        assert_eq!(fs::read_dir(&namespace).unwrap().count(), 0);

        let staging_id = Uuid::new_v4();
        let staging = namespace.join(format!("{GATEWAY_SERVICE_STAGING_PREFIX}{staging_id}"));
        fs::create_dir(&staging).expect("staging");
        fs::write(staging.join("partial"), b"partial").expect("partial");
        let outside = runtime.runtime_root.join("outside");
        fs::create_dir(&outside).expect("outside");
        fs::write(outside.join("keep"), b"keep").expect("outside sentinel");
        assert_eq!(runtime.cleanup_service_staging().unwrap(), 1);
        assert!(!staging.exists());
        assert!(outside.join("keep").exists());
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
