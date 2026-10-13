//! Explicit no-Git composition over the actual configured qualified catalogue.

use release_domain::ArtifactPath;
use run_domain::{Run, RunKind, RunState};
use run_orchestrator::{
    PreparedRunRuntime, RunRepository, RunRuntimeArtifactKind, RunRuntimeCatalog, RunRuntimeError,
    RunRuntimeInput,
};
use std::{collections::BTreeSet, fs, path::Path, sync::Arc};
use uuid::Uuid;
use volume_trait::RunVolumeStore;

use crate::{
    LocalRunRuntimeConfig, LocalRunRuntimeManager, MAX_RUNTIME_ARTIFACTS, MAX_RUNTIME_BYTES,
    context::HostContext,
    filesystem::{catalog, create_directory, runtime_error},
};

// Existing built-in guest state path; it is advertised only after the actual
// qualified full set proves this exact required writable state declaration.
const STATE_MOUNT: &str = "/var/lib/hephaestus";

#[derive(Clone)]
pub struct QualifiedInvocation {
    runs: Arc<dyn RunRepository>,
    volumes: Arc<dyn RunVolumeStore>,
}

impl LocalRunRuntimeManager {
    /// Initializes an explicit qualified no-Git Invocation materializer.
    ///
    /// These ports are composition data, not caller or provider/root authority.
    /// Trusted startup must validate managed roots and bind the SAME actual
    /// configured `RunPG` immutable repository and qualified119 catalogue, and
    /// SAME canonical volume store, used by execution. Per-Run persisted and
    /// live whole-set checks still run before staging writes. This constructor
    /// issues no credential/session and grants no provisioning permission.
    /// Ordinary construction and materialization remain unchanged.
    ///
    /// # Errors
    ///
    /// Rejects invalid roots through the existing initializer. Invocation
    /// preparation later denies absent/malformed lineage, config or live proofs.
    pub fn initialize_qualified_invocation(
        catalog: Arc<dyn RunRuntimeCatalog>,
        config: LocalRunRuntimeConfig,
        runs: Arc<dyn RunRepository>,
        volumes: Arc<dyn RunVolumeStore>,
    ) -> Result<Self, RunRuntimeError> {
        let mut manager = Self::initialize(catalog, config)?;
        manager.invocation = Some(QualifiedInvocation { runs, volumes });
        Ok(manager)
    }

    pub(crate) async fn invocation_run(
        &self,
        supplied: &Run,
    ) -> Result<Option<Run>, RunRuntimeError> {
        let Some(qualified) = &self.invocation else {
            // Default retains the old unsupported Invocation load below.
            return Ok(None);
        };
        let current = qualified
            .runs
            .get(supplied.id)
            .await
            .map_err(|_| runtime_error("exact persisted runtime Run unavailable"))?;
        if current.kind != RunKind::Invocation && supplied.kind != RunKind::Invocation {
            return Ok(None);
        }
        let vm_id = current.id.to_string();
        if &current != supplied
            || current.kind != RunKind::Invocation
            || current.state != RunState::Provisioning
            || current.vm_id.as_deref() != Some(vm_id.as_str())
            || current.attachment_id.is_some()
            || current.volume_id.is_some()
            || current.lease_id.is_some()
            || current.lease_fencing_token.is_some()
            || current.cancel_requested_at.is_some()
            || current.outcome.is_some()
            || current.exit.is_some()
            || current.failure.is_some()
        {
            return Err(runtime_error(
                "exact Provisioning Invocation runtime required",
            ));
        }
        Ok(Some(current))
    }

    pub(crate) async fn prepare_invocation(
        &self,
        run: &Run,
    ) -> Result<PreparedRunRuntime, RunRuntimeError> {
        let qualified = self
            .invocation
            .as_ref()
            .ok_or_else(|| runtime_error("qualified Invocation materializer unavailable"))?;
        if self.active_path(run.id).exists() {
            return Err(runtime_error("run runtime already exists"));
        }
        // Actual RunPG119 proves configured owner/profile/acquisition openness
        // and live immutable115/117 full held set, absent from the Run DTO. An
        // arbitrary catalogue implementation cannot attest these prerequisites.
        let input = self.catalog.load_runtime(run).await.map_err(catalog)?;
        validate_input(&input)?;
        let selections = qualified
            .volumes
            .load_run_selections(run.id)
            .await
            .map_err(|_| runtime_error("qualified runtime selections unavailable"))?;
        let identity = selections.identity();
        if identity.run_id() != run.id
            || identity.instance_id() != run.instance_id
            || identity.revision_id() != run.instance_revision_id
            || identity.release_id() != run.release_id
            || identity.release_agent_id() != run.release_agent_id
        {
            return Err(runtime_error(
                "runtime selections differ from exact persisted Run",
            ));
        }
        qualified
            .volumes
            .preflight_run(&selections)
            .await
            .map_err(|_| runtime_error("qualified runtime whole-set preflight denied"))?;
        let state_mount = if run.requires_state {
            if !selections.selections().iter().any(|selection| {
                let declaration = selection.declaration();
                declaration.slot().as_str() == "state"
                    && declaration.guest_path().as_str() == STATE_MOUNT
                    && declaration.access_mode().as_str() == "read_write"
                    && declaration.required()
            }) {
                return Err(runtime_error(
                    "actual qualified state declaration is missing",
                ));
            }
            Some(STATE_MOUNT)
        } else {
            None
        };
        let context = HostContext::invocation(run, &input, state_mount);
        // All persisted/catalogue/metadata/full-set checks above precede the
        // first staging write, including zero-slot runs. Shared mechanics still
        // rehash actual objects, seal readonly trees, atomically activate and
        // remove failed staging; destroy/recovery remain the existing boundary.
        let staging = self
            .config
            .runtime_root
            .join(format!(".prepare-{}", Uuid::new_v4()));
        create_directory(&staging, 0o700)?;
        let result = self.materialize_staging(run, &input, &staging, &context);
        if result.is_err() {
            let _cleanup = fs::remove_dir_all(&staging);
        }
        result
    }
}

fn validate_input(input: &RunRuntimeInput) -> Result<(), RunRuntimeError> {
    if !input.parameters.is_object()
        || input.artifacts.is_empty()
        || input.artifacts.len() > MAX_RUNTIME_ARTIFACTS
        || input.repository_id.is_some()
        || input.git_ref.is_some()
        || input.commit_sha.is_some()
        || input.update_id.is_some()
        || input.previous_revision_id.is_some()
        || input.previous_release_id.is_some()
        || input.previous_parameters.is_some()
        || !input.previous_artifacts.is_empty()
        || input.mailbox_event.is_some()
    {
        return Err(runtime_error(
            "Invocation catalogue contradicts no-Git runtime",
        ));
    }
    let mut bytes = 0_u64;
    let mut paths = BTreeSet::new();
    for artifact in &input.artifacts {
        ArtifactPath::parse(artifact.path.as_str())
            .map_err(|_| runtime_error("Invocation artifact path is invalid"))?;
        let mode = match artifact.kind {
            RunRuntimeArtifactKind::Executable => 0o555,
            RunRuntimeArtifactKind::File | RunRuntimeArtifactKind::Manifest => 0o444,
        };
        bytes = bytes
            .checked_add(artifact.size_bytes)
            .ok_or_else(|| runtime_error("Invocation artifact size is invalid"))?;
        if artifact.mode != mode
            || artifact.storage_key.is_nil()
            || bytes > MAX_RUNTIME_BYTES
            || !paths.insert(artifact.path.as_str())
            || artifact.path.chars().any(char::is_control)
            || artifact
                .path
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == "..")
        {
            return Err(runtime_error("Invocation artifact metadata is invalid"));
        }
    }
    // Avoid creating any staging tree for a file that is also another file's
    // ancestor; actual object hashing/regular-file checks remain shared IO.
    if paths.iter().any(|path| {
        Path::new(path).ancestors().skip(1).any(|parent| {
            parent
                .to_str()
                .is_some_and(|ancestor| paths.contains(ancestor))
        })
    }) {
        return Err(runtime_error(
            "Invocation artifact paths contradict the release tree",
        ));
    }
    Ok(())
}
