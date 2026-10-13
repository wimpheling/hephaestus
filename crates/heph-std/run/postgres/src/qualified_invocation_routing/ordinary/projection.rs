//! Closed ordinary comparison data; no controller choice, admission or IO permit.

use run_domain::{FreshRunExecutionProfile, RunState, StartRun};
use uuid::Uuid;
use vm_trait::{VmId, VmProviderOwnerScope};

/// Protected original consumption actually observed by this bounded reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedOrdinaryEvidence {
    /// Exact strict Legacy Git Normal birth, provision consumption and processed Start.
    LegacyProvisionConsumed,
}

/// Why no supported ordinary original or prospective unborn request was qualified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualifiedOrdinaryRoutingHoldReason {
    /// The acquired connection is not executing as the dedicated worker.
    WorkerRoleRequired,
    /// No matching globally persisted ordinary request exists.
    MissingRequest,
    /// Positive immutable evidence is missing, ambiguous or contradictory.
    ContradictoryOriginal,
    /// A Run-less request has placement, consumption or other original lineage.
    PartialUnbornLineage,
    /// Historical or unknown profile has no supported positive original proof.
    UnsupportedProfile,
    /// Ordinary canonical immutable qualification needs a separate worker facade.
    CanonicalOriginalFacadeRequired,
    /// Mailbox, Update or Invocation originals are outside this ordinary slice.
    UnsupportedProducer,
    /// The persisted original provider owner differs from the expected backend.
    ForeignOwner,
    /// The original has no consumption; Git Normal cannot have this valid birth shape.
    UnconsumedOriginal,
}

/// Closed snapshot comparison result, never an execution or cancellation permit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QualifiedOrdinaryRoutingDecision {
    /// One supported immutable original and its actual owner matched.
    Qualified(Box<QualifiedOrdinaryRoute>),
    /// A consistent real request/Start exists, without a durable Run or profile.
    ///
    /// Future trusted policy may act only at protected durable before-IO birth;
    /// this observed data grants no admission and must be re-read there.
    ProspectivelyUnborn(Box<QualifiedOrdinaryUnborn>),
    /// Preserve evidence/resources without assigning a family or trying a fallback.
    Held(QualifiedOrdinaryRoutingHoldReason),
}

/// Exact protected Git source/actor/request pins from a positive ordinary original.
///
/// For an unborn projection these are only currently observed request values:
/// no positive Run yet protects them. Historical actor/request IDs never become
/// authenticated principals, and cloning retains no database snapshot or lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedOrdinaryGitSource {
    pub(super) producer_request_id: Uuid,
    pub(super) repository_id: Uuid,
    pub(super) commit_sha: String,
    pub(super) git_ref: String,
    pub(super) receive_id: Option<Uuid>,
    pub(super) trigger_command_id: Option<Uuid>,
    pub(super) original_actor: Option<Uuid>,
    pub(super) original_request: Option<Uuid>,
    pub(super) retry_of_run_id: Option<Uuid>,
    pub(super) attempt: i32,
    pub(super) platform_policy_version: String,
}

impl QualifiedOrdinaryGitSource {
    /// Actual unique original producer row ID.
    #[must_use]
    pub const fn producer_request_id(&self) -> Uuid {
        self.producer_request_id
    }
    /// Actual original source repository, not a current authority grant.
    #[must_use]
    pub const fn repository_id(&self) -> Uuid {
        self.repository_id
    }
    /// Original exact commit from the producer.
    #[must_use]
    pub fn commit_sha(&self) -> &str {
        &self.commit_sha
    }
    /// Original exact Git ref from the producer.
    #[must_use]
    pub fn git_ref(&self) -> &str {
        &self.git_ref
    }
    /// Original receive when this was a Git receive producer.
    #[must_use]
    pub const fn receive_id(&self) -> Option<Uuid> {
        self.receive_id
    }
    /// Original trigger command when this was a command producer.
    #[must_use]
    pub const fn trigger_command_id(&self) -> Option<Uuid> {
        self.trigger_command_id
    }
    /// Historical actor for comparison only; absence creates no new identity.
    #[must_use]
    pub const fn original_actor(&self) -> Option<Uuid> {
        self.original_actor
    }
    /// Historical authenticated request ID, distinct from the producer row ID.
    #[must_use]
    pub const fn original_request(&self) -> Option<Uuid> {
        self.original_request
    }
    /// Original retry parent, when present.
    #[must_use]
    pub const fn retry_of_run_id(&self) -> Option<Uuid> {
        self.retry_of_run_id
    }
    /// Exact persisted producer attempt.
    #[must_use]
    pub const fn attempt(&self) -> i32 {
        self.attempt
    }
    /// Original platform version; future configuration never rewrites it.
    #[must_use]
    pub fn platform_policy_version(&self) -> &str {
        &self.platform_policy_version
    }
}

/// Qualified immutable original, with separately observed current lifecycle flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedOrdinaryRoute {
    pub(super) original_start: StartRun,
    pub(super) source: QualifiedOrdinaryGitSource,
    pub(super) owner: VmProviderOwnerScope,
    pub(super) vm_id: VmId,
    pub(super) project_id: Uuid,
    pub(super) release_contract_hash: [u8; 32],
    pub(super) evidence: QualifiedOrdinaryEvidence,
    pub(super) run_state: RunState,
    pub(super) cancellation_requested: bool,
    pub(super) acquisition_closed: bool,
}

impl QualifiedOrdinaryRoute {
    /// Exact Start independently compared with original Run, request and inbox.
    #[must_use]
    pub const fn original_start(&self) -> &StartRun {
        &self.original_start
    }
    /// Protected original Git producer/source data.
    #[must_use]
    pub const fn source(&self) -> &QualifiedOrdinaryGitSource {
        &self.source
    }
    /// Original persisted profile; no current policy choice is made.
    #[must_use]
    pub const fn profile(&self) -> FreshRunExecutionProfile {
        FreshRunExecutionProfile::LegacyScalar
    }
    /// Actual persisted owner matched to the checked expected backend.
    #[must_use]
    pub const fn owner(&self) -> &VmProviderOwnerScope {
        &self.owner
    }
    /// Actual protected placement VM ID, never a missing-VM fallback.
    #[must_use]
    pub const fn vm_id(&self) -> &VmId {
        &self.vm_id
    }
    /// Original project checked by the protected immutable placement helper.
    #[must_use]
    pub const fn project_id(&self) -> Uuid {
        self.project_id
    }
    /// Original release contract hash checked by that same helper.
    #[must_use]
    pub const fn release_contract_hash(&self) -> &[u8; 32] {
        &self.release_contract_hash
    }
    /// Actual original consumption evidence.
    #[must_use]
    pub const fn evidence(&self) -> QualifiedOrdinaryEvidence {
        self.evidence
    }
    /// Current observed lifecycle state, without live dispatch qualification.
    #[must_use]
    pub const fn run_state(&self) -> RunState {
        self.run_state
    }
    /// Current observed cancellation flag.
    #[must_use]
    pub const fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }
    /// Current observed acquisition closure flag.
    #[must_use]
    pub const fn acquisition_closed(&self) -> bool {
        self.acquisition_closed
    }
}

/// Currently observed Run-less request/Start; deliberately no profile or owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QualifiedOrdinaryUnborn {
    pub(super) start: StartRun,
    pub(super) source: QualifiedOrdinaryGitSource,
}

impl QualifiedOrdinaryUnborn {
    /// Exact request/actual outbox Start comparison, not an admitted original Run.
    #[must_use]
    pub const fn start(&self) -> &StartRun {
        &self.start
    }
    /// Current producer/source values, not yet protected by a positive Run.
    #[must_use]
    pub const fn source(&self) -> &QualifiedOrdinaryGitSource {
        &self.source
    }
}
