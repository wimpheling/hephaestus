//! Observations only; no status is a physical or database receipt.

/// Process-owned managed preparation/closing phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedSupervisorPhase {
    /// No managed preparation has registered.
    Empty,
    /// Read-only preparation or physical acquisition is owned by the service.
    Preparing,
    /// The original tuple is retained through actual controller construction.
    Constructing,
    /// Assembly completed; recovery and readiness remain unwired.
    PendingRecovery,
    /// Qualified original-family recovery passed; listeners have not become Ready.
    Recovered,
    /// Actual listener/consumer signals passed while intake remains open.
    Ready,
    /// Intake is sealed; a supported physical cleanup pass is running.
    Closing,
    /// Original custody remains, with explicit failure or incomplete evidence.
    Held,
    /// No originals were acquired, or their supported cleanup and service settlement passed.
    Stopped,
}

/// Why service custody remains held.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ManagedHeldReason {
    /// A read-only preparation gate failed before acquisition.
    Preparation,
    /// Partial acquisition has no safe generic resume/dispose port.
    Acquisition,
    /// Actual same-owner physical cleanup failed.
    PhysicalCleanup,
    /// Physical stop does not establish qualified per-Run reconciliation.
    ReconciliationUnavailable,
    /// Running-service joining or retained transport settlement is incomplete.
    Services,
    /// Separate ordinary Build/OCI/Gateway cleanup has no qualified proof port yet.
    OrdinaryServicesCleanupUnavailable,
    /// A service-owned task ended unexpectedly.
    TaskFailure,
}

/// Read-only service diagnostics, never owners or readiness authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManagedSupervisorStatus {
    /// Current service phase.
    pub phase: ManagedSupervisorPhase,
    /// Actual physical cleanup passes attempted, including explicit retries.
    pub cleanup_attempts: u64,
    /// An owning preparation or cleanup job is still active.
    pub in_flight: bool,
    /// Actual controller assembly completed; this does not publish Ready.
    pub assembled: bool,
    /// Current held boundary, if any.
    pub held_reason: Option<ManagedHeldReason>,
    /// Last observed error description; actual error objects remain private.
    pub last_error: Option<String>,
}

impl Default for ManagedSupervisorStatus {
    fn default() -> Self {
        Self {
            phase: ManagedSupervisorPhase::Empty,
            cleanup_attempts: 0,
            in_flight: false,
            assembled: false,
            held_reason: None,
            last_error: None,
        }
    }
}
