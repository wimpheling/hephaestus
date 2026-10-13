//! Two-VM build and verification operation.

use super::{
    constants::{VerifiedVmOciOutput, VmOciOperationConfig},
    filesystem::initialize_private_directory,
    output::validate_executable,
    specs::roots_overlap,
};
use oci_builder_worker::{IsolatedOciBuild, OciWorkerError};
use std::{fs, path::PathBuf, sync::Arc};
use vm_trait::{RootFilesystem, VmProvider, VmResources};

/// Executes a repository OCI build and independent verification in fresh VMs.
///
/// Publication deliberately remains outside this adapter: neither VM receives
/// a registry token, and a caller must validate these outputs before issuing a
/// host-controlled publication credential.
#[derive(Clone)]
pub struct VmOciOperation {
    pub(super) provider: Arc<dyn VmProvider>,
    pub(super) builder_root: RootFilesystem,
    pub(super) verifier_root: RootFilesystem,
    pub(super) candidate_root: PathBuf,
    pub(super) scratch_root: PathBuf,
    pub(super) mkfs_ext4: PathBuf,
    pub(super) verification_root: PathBuf,
    pub(super) resources: VmResources,
    pub(super) workload_phase_timing: bool,
    pub(super) custody: Arc<crate::owner::Owner>,
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
        let custody = Arc::new(crate::owner::Owner::initialize(
            &config.candidate_root,
            &config.scratch_root,
            &config.verification_root,
            &config.mkfs_ext4,
        )?);
        Ok(Self {
            custody,
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

    /// Runs one original registered operation; cancellation drops only waiter.
    /// Same-ID redelivery reuses exact original inputs/outcome, never reprovisions.
    /// # Errors
    /// Returns original safe failure classes; unresolved original custody stays retained.
    pub async fn execute(
        &self,
        request: &IsolatedOciBuild,
    ) -> Result<VerifiedVmOciOutput, OciWorkerError> {
        crate::owner::execute(self.clone(), request.clone()).await
    }
}
