//! Shared OCI operation constants and public configuration types.

use builder_catalog_domain::OciDigest;
use std::{path::PathBuf, time::Instant};
use vm_trait::{RootFilesystem, VmResources};

pub const TRUSTED_SYSTEM_PATH: &str =
    "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
pub const BUILDER_SOURCE_GUEST_PATH: &str = "/workspace/source";
pub const BUILDER_BASE_GUEST_PATH: &str = "/workspace/heph-base";
pub const BUILDER_OUTPUT_GUEST_PATH: &str = "/workspace/candidate";
pub const BUILDER_SCRATCH_GUEST_PATH: &str = "/workspace/buildah";
pub const BUILDER_SCRATCH_DISK_ID: &str = "repository-oci-scratch";
// Buildah can hold the imported base, the working container, and the exported
// layer at once. Keep the job-scoped ext4 disk sparse on the host, but give the
// guest enough capacity for that bounded copy-on-write peak.
pub const BUILDER_SCRATCH_BYTES: u64 = 8 * 1024 * 1024 * 1024;
pub const VERIFIER_OUTPUT_GUEST_PATH: &str = "/workspace/verification";
pub const PLATFORM_OCI_BUILDER_ENV: &str = "HEPH_PLATFORM_OCI_BUILDER";
pub const PLATFORM_OCI_VERIFIER_ENV: &str = "HEPH_PLATFORM_OCI_VERIFIER";
pub const VERIFIER_TRIVY_CACHE_ENV: &str = "TRIVY_CACHE_DIR";
pub const VERIFIER_TRIVY_CACHE_PATH: &str = "/workspace/verification/trivy-cache";
pub const VERIFIER_SYFT_UPDATE_ENV: &str = "SYFT_CHECK_FOR_APP_UPDATE";
pub const VERIFIER_SYFT_CACHE_ENV: &str = "XDG_CACHE_HOME";
pub const VERIFIER_SYFT_CACHE_PATH: &str = "/workspace/verification/syft-cache";
pub const WORKLOAD_PHASE_TIMING_EVENT: &str = "phase-timing";
pub const WORKLOAD_PHASE_TIMING_MAX_MS: u128 = 45 * 60 * 1_000;

/// Emits only a bounded workload measurement.  The outer GCP supervisor owns
/// acceptance; these values help locate cost inside the untrusted workload
/// and are therefore explicitly informational.
pub struct WorkloadPhaseTimer {
    pub phase: &'static str,
    pub started: Instant,
    pub enabled: bool,
    pub finished: bool,
}

impl WorkloadPhaseTimer {
    pub fn start(phase: &'static str, enabled: bool) -> Self {
        Self {
            phase,
            started: Instant::now(),
            enabled,
            finished: false,
        }
    }

    pub fn finish(mut self, success: bool) {
        self.finished = true;
        self.emit(if success { "passed" } else { "failed" });
    }

    pub fn emit(&self, status: &'static str) {
        if !self.enabled {
            return;
        }
        let elapsed_ms = self.started.elapsed().as_millis();
        let duration_ms = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            elapsed_ms
        } else {
            0
        };
        let status = if elapsed_ms <= WORKLOAD_PHASE_TIMING_MAX_MS {
            status
        } else {
            "unknown"
        };
        eprintln!(
            "HEPH_GCP_COOKING event={WORKLOAD_PHASE_TIMING_EVENT} phase={} status={status} duration_ms={duration_ms}",
            self.phase
        );
    }
}

impl Drop for WorkloadPhaseTimer {
    fn drop(&mut self) {
        if !self.finished {
            self.emit("unknown");
        }
    }
}

/// Fixed local roots and VM resources for repository OCI operations.
///
/// The builder and verifier roots are platform-operation images selected by
/// the local operator, never repository metadata.
#[derive(Debug, Clone)]
pub struct VmOciOperationConfig {
    /// Platform-owned root for the one-shot Buildah guest.
    pub builder_root: RootFilesystem,
    /// Platform-owned root for the independent verifier guest.
    pub verifier_root: RootFilesystem,
    /// Private root containing one candidate layout per durable preparation.
    pub candidate_root: PathBuf,
    /// Private root containing one transient Buildah store per preparation.
    pub scratch_root: PathBuf,
    /// Absolute administrator-owned mke2fs-compatible executable.
    pub mkfs_ext4: PathBuf,
    /// Private root containing verifier evidence and exported roots.
    pub verification_root: PathBuf,
    /// Fixed bounded resources for each operational guest.
    pub resources: VmResources,
    /// Whether the workload may emit bounded informational phase timings.
    pub workload_phase_timing: bool,
}

/// Verified local outputs produced by distinct builder and verifier VMs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedVmOciOutput {
    /// Sealed candidate OCI layout mounted read-only into the verifier.
    pub layout: PathBuf,
    /// Verifier-created SPDX SBOM.
    pub sbom: PathBuf,
    /// Verifier-created offline vulnerability report.
    pub scan: PathBuf,
    /// Verifier-exported root filesystem for later atomic materialization.
    pub rootfs: PathBuf,
    /// Exact OCI manifest digest observed by the verifier.
    pub manifest_digest: OciDigest,
}

#[derive(Debug)]
pub struct ScratchDisk {
    pub path: PathBuf,
    pub filesystem_uuid: uuid::Uuid,
}
