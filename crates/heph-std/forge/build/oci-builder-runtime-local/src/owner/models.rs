use crate::VerifiedVmOciOutput;
use oci_builder_worker::OciWorkerError;
use std::sync::Arc;
/// Comparison-only phase of an original live OCI VM operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OciCleanupPhase {
    /// Original preparation/work/watcher/cleanup still needs actual join.
    Executing,
    /// Original VM attempt detach remains unresolved.
    VmHeld,
    /// Original path/FD/acquisition or cleanup remains unresolved.
    FilesystemHeld,
    /// Parent mkfs settled, but configured helper profile is unqualified.
    ScratchHelperUnqualified,
    /// Original live operation settled; immutable output remains pinned.
    Settled,
}
/// Nonsecret observation; not a worker/native/global receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciCleanupStatus {
    /// Exact original durable job.
    pub job_id: uuid::Uuid,
    /// Actual observed cleanup phase.
    pub phase: OciCleanupPhase,
    /// Candidate/verification roots deliberately retained for publication.
    pub retained_outputs: u8,
    /// Number of retained original errors, without exposing diagnostics.
    pub retained_errors: usize,
}
/// Observation unresolved; original tasks/FDs/VMs stay with the operation.
#[derive(Debug, thiserror::Error)]
#[error("original OCI operation cleanup remains held")]
pub struct OciCleanupHeld {
    /// Exact original unsettled observations.
    pub remaining: Vec<OciCleanupStatus>,
    /// Configured root/program identity failed its retained FD recheck.
    pub roots_held: bool,
}
pub(super) type Outcome = Result<VerifiedVmOciOutput, Arc<OciWorkerError>>;
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Phase {
    Builder,
    Verifier,
}
