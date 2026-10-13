//! Explicit owning composition for one managed qualified Invocation controller.

mod assemble;
mod roots;
mod validation;

use crate::{AppError, RuntimePolicy, component};
use control_plane_postgres::ControlPlanePool;
use heph_run::{CancelRun, RepositoryError, Run, RunRepository};
use heph_runtime::RootFilesystem;
use release_service::InstanceExecutionConfiguration;
use run_orchestrator::{
    OrchestratorError, RunCleanupWakeProgress, RunCleanupWakeRequest, RunOperationGuards,
    RunOrchestrator,
};
use run_postgres::PgRunRepository;
use run_runtime_local::LocalRunRuntimeConfig;
use runtime_types::RunId;
use secret_key_local::LocalKeyProvider;
use secret_runtime::EphemeralSecretConfig;
use std::{collections::BTreeMap, path::PathBuf, sync::Arc, time::Duration};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::{LocalVolumeConfig, VolumeRootOwner};

/// The three actual independent pools supplied by the composition owner.
pub struct ManagedInvocationPools {
    /// Established owner administrator pool; used only by startup preflight here.
    pub admin: ControlPlanePool,
    /// Dedicated worker pool selecting `hephaestus_worker`.
    pub worker: ControlPlanePool,
    /// Distinct actor/application pool selecting `hephaestus_app`.
    pub application: ControlPlanePool,
}

/// Host-loaded authority storage and positive runtime session lifetime.
pub struct ManagedInvocationAuthority {
    /// Dedicated canonical host-private handoff directory.
    pub root: PathBuf,
    /// Existing nonzero host-loaded encryption key; never a guest credential.
    pub key: [u8; 32],
    /// Positive, representable runtime session lifetime.
    pub session_ttl: Duration,
}

/// Trusted inputs retained by the owning controller, with no provider bootstrap.
///
/// Supply the SAME actual typed provider, physical volume owner and guard registry
/// retained by managed backend startup. Configuration values grant no actor rights.
/// Every controller for this provider clone family must share `operation_guards`.
pub struct ManagedInvocationInputs {
    /// SAME existing concrete managed provider; arbitrary provider ports are excluded.
    pub provider: Arc<LibkrunProvider>,
    /// Full trusted configuration that must equal the provider's retained configuration.
    pub expected_provider: LibkrunConfig,
    /// SAME existing descriptor owner for the physical persistent volume root.
    pub volume_owner: Arc<VolumeRootOwner>,
    /// Checked immutable execution stamp shared by all qualified adapters.
    pub configuration: InstanceExecutionConfiguration,
    /// Actual owner, dedicated worker and distinct application pools.
    pub pools: ManagedInvocationPools,
    /// Current local volume roots, host, executable and lease configuration.
    pub local_volume: LocalVolumeConfig,
    /// Current transient runtime and immutable release object roots.
    pub run_runtime: LocalRunRuntimeConfig,
    /// Digest-pinned OCI references mapped to actual materialized roots.
    pub root_images: BTreeMap<String, RootFilesystem>,
    /// Current policy; its exact unclamped mapping must equal the checked stamp.
    pub runtime_policy: RuntimePolicy,
    /// Existing checked host key provider for the secret adapters.
    pub secret_keys: LocalKeyProvider,
    /// Existing private ephemeral root and its explicit memory-filesystem requirement.
    pub secret_mounts: EphemeralSecretConfig,
    /// Existing runtime authority handoff tuple and lifetime.
    pub authority: ManagedInvocationAuthority,
    /// Positive supported capacity for the underlying instance-state port.
    pub instance_state_capacity_bytes: u64,
    /// Positive canonical cleanup deadline, at most the existing 30-second maximum.
    pub cleanup_timeout: Duration,
    /// SAME supplied registry, installed unchanged through `with_operation_guards`.
    pub operation_guards: Arc<RunOperationGuards>,
}

/// Owns one production qualified Invocation orchestrator and its exact inputs.
///
/// Construction starts no background task and offers no raw provider getter.
/// Explicit Cancel/restart operations retain this original controller in their
/// owning task. They grant no routing, global shutdown, adoption or gate reopening.
/// Drop establishes no stopped or cleaned state. The caller retains backend
/// custody through construction errors/cancellation and all later lifecycle work.
pub struct ManagedInvocationController {
    retained: ManagedInvocationInputs,
    runs: Arc<PgRunRepository>,
    orchestrator: Arc<RunOrchestrator>,
}

impl ManagedInvocationController {
    /// Validates and composes the supplied actual backend before execution use.
    ///
    /// All configuration/root/owner checks and the accepted three-pool startup
    /// preflight precede the first filesystem initializer. Checks are repeated
    /// after the awaited preflight. Qualified per-Run checks still govern every
    /// later effect; startup success grants neither actor rights nor readiness.
    ///
    /// # Errors
    /// Rejects wrong providers/owners/configuration, unsafe or incompatible roots,
    /// roles/schema/service drift, or a failed production filesystem initializer.
    /// Initializer failures may leave directories; no automatic cleanup is claimed.
    pub async fn new(inputs: ManagedInvocationInputs) -> Result<Self, AppError> {
        validation::validate(&inputs)?;
        control_plane_postgres::verify_qualified_invocation_contract(
            &inputs.pools.admin,
            &inputs.pools.worker,
            &inputs.pools.application,
        )
        .await
        .map_err(component("qualified Invocation database startup"))?;
        validation::validate(&inputs)?;
        let (runs, orchestrator) = assemble::build(&inputs)?;
        Ok(Self {
            retained: inputs,
            runs,
            orchestrator,
        })
    }

    /// Borrows the composed operation handle while this owning controller is retained.
    #[must_use]
    pub fn orchestrator(&self) -> &RunOrchestrator {
        &self.orchestrator
    }

    /// Delivers actual cancellation through this SAME retained controller.
    ///
    /// The exact command and original controller enter owning execution before
    /// persistence or cleanup. Dropping the waiter never aborts admitted work;
    /// accepted replay still reconciles queued cleanup. Errors are logged even
    /// if the waiter disappears. Success is not an instance-wide Drain receipt.
    ///
    /// # Errors
    /// Propagates original admission, prepared-history or actual cleanup errors.
    pub async fn cancel_run(
        self: Arc<Self>,
        command: &CancelRun,
    ) -> Result<bool, OrchestratorError> {
        let command = command.clone();
        tokio::spawn(async move {
            let result = async {
                self.validate_owners()
                    .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
                self.orchestrator.cancel_run(&command).await
            }.await;
            if let Err(error) = &result {
                tracing::error!(run_id=%command.run_id, %error, "owned Invocation cancellation remains held");
            }
            drop(self);
            result
        }).await.map_err(|error| RepositoryError::Storage(Box::new(error)))?
    }

    /// Reconciles the complete qualified Invocation inventory with original custody.
    ///
    /// Caller cancellation drops only its waiter. No global sweep or count
    /// proves prior-process physical absence, full readiness or recipe Drain.
    ///
    /// # Errors
    /// Every unknown original, held operation or incomplete cleanup returns error.
    pub async fn recover_after_restart(self: Arc<Self>) -> Result<usize, OrchestratorError> {
        tokio::spawn(async move {
            let result = async {
                self.validate_owners()
                    .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
                self.orchestrator.recover_after_restart().await
            }
            .await;
            if let Err(error) = &result {
                tracing::error!(%error, "owned Invocation restart recovery remains held");
            }
            drop(self);
            result
        })
        .await
        .map_err(|error| RepositoryError::Storage(Box::new(error)))?
    }

    /// Wakes exact cancelled cleanup through this SAME private orchestrator.
    ///
    /// Retains this owning controller through the awaited Wake call. The existing
    /// core API requalifies original scope and cleanup evidence; its progress is
    /// neither a Drain proof nor a receipt. This delegate starts no additional task.
    ///
    /// # Errors
    /// Propagates the existing owning Wake qualification or orchestration failure.
    pub async fn wake_cleanup(
        self: Arc<Self>,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        let result = Arc::clone(&self.orchestrator).wake_cleanup(request).await;
        drop(self);
        result
    }

    /// Reads one persisted Run through the SAME configured repository/catalog.
    ///
    /// This exposes no repository builder, pool or mutable composition port.
    ///
    /// # Errors
    /// Propagates missing, malformed or unavailable persisted Run data.
    pub async fn run(&self, id: RunId) -> Result<Run, RepositoryError> {
        self.runs.get(id).await
    }

    /// Borrows only this original controller's retained composition inside App.
    pub(super) const fn drain_inputs(&self) -> &ManagedInvocationInputs {
        &self.retained
    }

    /// Positively compares every native Drain row before any delivery in the batch.
    ///
    /// Snapshots are separate; the native reader and later Cancel still independently
    /// requalify. This comparison neither synthesizes a DTO nor proves physical Drain.
    pub(super) async fn qualify_instance_drain_batch(
        &self,
        target: &recipe_application::PreparedInstanceSafetyTarget,
        rows: &[run_postgres::InstanceDrainRunObservation],
    ) -> Result<(), OrchestratorError> {
        self.validate_owners()
            .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
        let configuration = &self.retained.configuration;
        if target.provider_namespace() != configuration.provider_scope().namespace()
            || target.host_id() != configuration.provider_scope().host_id()
        {
            return Err(RepositoryError::InvalidData("instance Drain target owner differs").into());
        }
        let reader = run_postgres::PgQualifiedInvocationRoutingReader::new(
            self.retained.pools.worker.clone(),
            configuration.clone(),
        );
        let bytes = serde_json::to_vec(&configuration.canonical_json())
            .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
        for row in rows {
            let run_postgres::QualifiedInvocationRoutingDecision::Qualified(route) =
                reader.inspect_original(row.run_id).await?
            else {
                return Err(
                    RepositoryError::InvalidData("instance Drain original remains held").into(),
                );
            };
            let original = route.original_start();
            if route.owner() != configuration.provider_scope()
                || route.configuration_bytes() != bytes
                || original.run_id != row.run_id
                || original.instance_id != target.prepared().instance_id()
                || original.instance_revision_id != target.prepared().revision_id()
                || original.release_id != target.prepared().source().pin().release_id
                || original.release_agent_id != target.prepared().source().pin().release_agent_id
                || row
                    .cancel
                    .as_ref()
                    .is_some_and(|cancel| cancel.run_id != row.run_id)
            {
                return Err(RepositoryError::InvalidData(
                    "instance Drain original controller differs",
                )
                .into());
            }
        }
        self.validate_owners()
            .map_err(|error| RepositoryError::Storage(Box::new(error)))?;
        Ok(())
    }

    /// Rechecks retained owner/configuration bindings without initializing roots.
    ///
    /// This is not database preflight, physical readiness, or ongoing attestation.
    ///
    /// # Errors
    /// Rejects replaced roots/owners or changed host configuration.
    pub fn validate_owners(&self) -> Result<(), AppError> {
        validation::validate(&self.retained)
    }
}
