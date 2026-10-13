use run_domain::{RunState, StartRun};
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

/// Which immutable Invocation evidence was actually observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedInvocationEvidence {
    /// Protected original115 intent without an actual117 consumption.
    ///
    /// This label describes evidence, not the Run's current lifecycle state.
    /// Cancelled, closed and cleanup-completed originals can retain this intent.
    QueuedIntent115,
    /// The actual immutable117 consumption and processed original Start match.
    Consumed117,
}

/// Why this initial reader cannot identify a qualified Invocation original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedInvocationRoutingHoldReason {
    /// The acquired connection is not executing as the configured worker role.
    WorkerRoleRequired,
    /// No globally selected Run exists; fresh Normal profile choice is unresolved.
    MissingRun,
    /// The persisted profile is unknown, historical or not owned canonical.
    UnsupportedProfile,
    /// The persisted producer is outside this initial Invocation-only reader.
    UnsupportedProducer,
    /// Original Run, producer, Start/inbox or actual consumption contradicts.
    ContradictoryOriginal,
    /// The actual persisted namespace/host differs from the expected owner.
    ForeignOwner,
    /// Exact original configuration bytes or their encoding/hash differ.
    ConfigurationMismatch,
}

/// Closed comparison result; neither variant is an IO permit or routing authority.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedInvocationRoutingDecision {
    /// All supported immutable original comparison checks matched one snapshot.
    Qualified(Box<QualifiedInvocationRoute>),
    /// Preserve the original and its resources without guessing another family.
    Held(QualifiedInvocationRoutingHoldReason),
}

/// Immutable original comparison projection with a separately observed Run state.
///
/// Fields cannot be supplied by a caller. Owning composition must still match its
/// retained concrete controller/backend and requalify every effect. Cloning this
/// data does not retain a live database snapshot, lock, grant or controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedInvocationRoute {
    pub(super) original_start: StartRun,
    pub(super) invocation_id: Uuid,
    pub(super) original_actor: Uuid,
    pub(super) original_request: Uuid,
    pub(super) command_key: [u8; 32],
    pub(super) input_hash: [u8; 32],
    pub(super) configuration_bytes: Vec<u8>,
    pub(super) owner: VmProviderOwnerScope,
    pub(super) evidence: QualifiedInvocationEvidence,
    pub(super) run_state: RunState,
    pub(super) cancellation_requested: bool,
    pub(super) acquisition_closed: bool,
}

impl QualifiedInvocationRoute {
    /// Exact persisted original Start, independently compared with Run/producer.
    #[must_use]
    pub const fn original_start(&self) -> &StartRun {
        &self.original_start
    }

    /// Unique protected original Invocation identifier.
    #[must_use]
    pub const fn invocation_id(&self) -> Uuid {
        self.invocation_id
    }

    /// Original actor identifier for comparison only, never a current identity.
    #[must_use]
    pub const fn original_actor(&self) -> Uuid {
        self.original_actor
    }

    /// Original authenticated request identifier for comparison only.
    #[must_use]
    pub const fn original_request(&self) -> Uuid {
        self.original_request
    }

    /// Original immutable invocation command key.
    #[must_use]
    pub const fn command_key(&self) -> &[u8; 32] {
        &self.command_key
    }

    /// Original immutable producer input hash checked by the existing SQL graph.
    #[must_use]
    pub const fn input_hash(&self) -> &[u8; 32] {
        &self.input_hash
    }

    /// Exact original configuration bytes matched to the expected canonical bytes.
    #[must_use]
    pub fn configuration_bytes(&self) -> &[u8] {
        &self.configuration_bytes
    }

    /// Persisted provider namespace/host matched to the expected checked owner.
    #[must_use]
    pub const fn owner(&self) -> &VmProviderOwnerScope {
        &self.owner
    }

    /// Whether original115 intent or actual117 consumption was observed.
    #[must_use]
    pub const fn evidence(&self) -> QualifiedInvocationEvidence {
        self.evidence
    }

    /// Current Run state in that snapshot; no dispatch eligibility is implied.
    #[must_use]
    pub const fn run_state(&self) -> RunState {
        self.run_state
    }

    /// Whether durable cancellation was already requested in that snapshot.
    #[must_use]
    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }

    /// Whether Run acquisition was permanently closed in that snapshot.
    #[must_use]
    pub const fn acquisition_closed(&self) -> bool {
        self.acquisition_closed
    }
}
