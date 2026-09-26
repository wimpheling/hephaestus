//! Two-VM build and verification operation.

use super::{
    constants::{ScratchDisk, VerifiedVmOciOutput, VmOciOperationConfig, WorkloadPhaseTimer},
    filesystem::{
        initialize_private_directory, prepare_empty_directory, prepare_scratch_disk,
        remove_private_directory, seal_candidate_layout, symlinked_tree, verified_vm_output,
    },
    guest::{guest_failure_phase, operation_phase},
    output::{normalize_layout_to_single_index, validate_executable},
    specs::roots_overlap,
    specs::{builder_vm_spec, verifier_vm_spec},
};
use oci_builder_worker::{IsolatedOciBuild, OciWorkerError};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};
use vm_trait::{RootFilesystem, VmProvider, VmResources, VmSpec};

/// Executes a repository OCI build and independent verification in fresh VMs.
///
/// Publication deliberately remains outside this adapter: neither VM receives
/// a registry token, and a caller must validate these outputs before issuing a
/// host-controlled publication credential.
#[derive(Clone)]
pub struct VmOciOperation {
    provider: Arc<dyn VmProvider>,
    builder_root: RootFilesystem,
    verifier_root: RootFilesystem,
    candidate_root: PathBuf,
    scratch_root: PathBuf,
    mkfs_ext4: PathBuf,
    verification_root: PathBuf,
    resources: VmResources,
    workload_phase_timing: bool,
}

impl VmOciOperation {
    /// Validates private roots and creates a reusable VM operation boundary.
    ///
    /// # Errors
    ///
    /// Returns an error for unsafe roots or zero operational resources.
    pub fn initialize(
        provider: Arc<dyn VmProvider>,
        mut config: VmOciOperationConfig,
    ) -> Result<Self, OciWorkerError> {
        if config.resources.vcpus == 0
            || config.resources.memory_mib == 0
            || !config.candidate_root.is_absolute()
            || !config.scratch_root.is_absolute()
            || !config.verification_root.is_absolute()
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        config.candidate_root = initialize_private_directory(&config.candidate_root)?;
        config.scratch_root = initialize_private_directory(&config.scratch_root)?;
        validate_executable(&config.mkfs_ext4)?;
        config.mkfs_ext4 =
            fs::canonicalize(&config.mkfs_ext4).map_err(OciWorkerError::Filesystem)?;
        config.verification_root = initialize_private_directory(&config.verification_root)?;
        if roots_overlap(&config.candidate_root, &config.scratch_root)
            || roots_overlap(&config.candidate_root, &config.verification_root)
            || roots_overlap(&config.scratch_root, &config.verification_root)
        {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        Ok(Self {
            provider,
            builder_root: config.builder_root,
            verifier_root: config.verifier_root,
            candidate_root: config.candidate_root,
            scratch_root: config.scratch_root,
            mkfs_ext4: config.mkfs_ext4,
            verification_root: config.verification_root,
            resources: config.resources,
            workload_phase_timing: config.workload_phase_timing,
        })
    }

    /// Builds and verifies one exact request using two separate VMs.
    ///
    /// # Errors
    ///
    /// Returns a safe worker error when either guest cannot prove a successful
    /// bounded operation or creates malformed output.
    pub async fn execute(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<VerifiedVmOciOutput, OciWorkerError> {
        if !request.network_disabled || !request.ambient_credentials_disabled {
            return Err(OciWorkerError::InvalidConfiguration);
        }
        let candidate = self.candidate_root.join(request.job_id.to_string());
        let scratch = prepare_scratch_disk(&self.scratch_root, &self.mkfs_ext4, request.job_id)?;
        let verification = self.verification_root.join(request.job_id.to_string());
        prepare_empty_directory(&candidate)?;
        prepare_empty_directory(&verification)?;
        let result = async {
            self.run_builder(request, &candidate, &scratch).await?;
            // The builder emits the standard OCI outer-index-to-manifest form.
            // Convert that untrusted output into the one platform index the
            // publisher accepts, before it becomes an immutable verifier input.
            if symlinked_tree(&candidate)? {
                return Err(OciWorkerError::InvalidOutput);
            }
            normalize_layout_to_single_index(&candidate)?;
            seal_candidate_layout(&candidate)?;
            self.run_verifier(request, &candidate, &verification)
                .await?;
            verified_vm_output(&candidate, &verification)
        }
        .await;
        if result.is_err() {
            let _ignored = remove_private_directory(&candidate);
            let _ignored = fs::remove_file(&scratch.path);
            let _ignored = remove_private_directory(&verification);
        } else {
            let _ignored = fs::remove_file(&scratch.path);
        }
        result
    }

    async fn run_builder(
        &self,
        request: &IsolatedOciBuild,
        candidate: &Path,
        scratch: &ScratchDisk,
    ) -> Result<(), OciWorkerError> {
        self.run_guest(
            "builder",
            builder_vm_spec(
                request,
                candidate,
                scratch,
                &self.builder_root,
                &self.resources,
            )?,
        )
        .await
    }

    async fn run_verifier(
        &self,
        request: &IsolatedOciBuild,
        candidate: &Path,
        verification: &Path,
    ) -> Result<(), OciWorkerError> {
        self.run_guest(
            "verifier",
            verifier_vm_spec(
                request,
                candidate,
                verification,
                &self.verifier_root,
                &self.resources,
            ),
        )
        .await
    }

    async fn run_guest(&self, phase: &'static str, spec: VmSpec) -> Result<(), OciWorkerError> {
        let timing_phase = match phase {
            "builder" => "oci-builder",
            "verifier" => "oci-verifier",
            _ => return self.run_guest_inner(phase, spec).await,
        };
        let timer = WorkloadPhaseTimer::start(timing_phase, self.workload_phase_timing);
        let result = self.run_guest_inner(phase, spec).await;
        timer.finish(result.is_ok());
        result
    }

    async fn run_guest_inner(
        &self,
        phase: &'static str,
        spec: VmSpec,
    ) -> Result<(), OciWorkerError> {
        let Ok(instance) = self.provider.provision(spec).await else {
            // Provider errors may wrap arbitrary source text. Preserve only
            // the fixed operation phase at this logging boundary.
            return Err(OciWorkerError::IsolatedVmFailed {
                phase,
                exit_code: None,
            });
        };
        let mut events = instance.subscribe_events();
        if instance.start().await.is_err() {
            let _ignored = instance.destroy().await;
            return Err(OciWorkerError::IsolatedVmFailed {
                phase: operation_phase(phase, "startup"),
                exit_code: None,
            });
        }
        let exit = instance.wait().await.ok();
        let completed = exit
            .as_ref()
            .is_some_and(|exit| exit.code == Some(0) && exit.signal.is_none());
        if !completed {
            eprintln!(
                "isolated OCI {phase} guest exit code={:?} signal={:?}",
                exit.as_ref().and_then(|value| value.code),
                exit.as_ref().and_then(|value| value.signal),
            );
        }
        let failure_phase = if completed {
            None
        } else {
            // `wait` is satisfied as soon as the provider records the terminal
            // status. The independent guest-control task can still be
            // forwarding the final bounded log frames. Wait only until its
            // terminal event (or the short deadline), reduce those bytes to a
            // fixed safe phase, and discard them before any persistence.
            Some(guest_failure_phase(phase, &mut events).await)
        };
        let destroyed = instance.destroy().await.is_ok();
        if !destroyed || !completed {
            return Err(OciWorkerError::IsolatedVmFailed {
                phase: failure_phase.unwrap_or_else(|| operation_phase(phase, "execution")),
                exit_code: exit.and_then(|exit| exit.code),
            });
        }
        Ok(())
    }
}
