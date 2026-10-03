use std::collections::BTreeMap;

use async_trait::async_trait;
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;

use crate::{
    AttemptProvenance, CommandIdentity, CommandReceipt, DeploymentAttemptId, DeploymentError,
    DeploymentId, DeploymentIntent, DeploymentLifecycle, DiagnosticCode, EffectClaim,
    EffectEvidence, InstallDeployment, InstallProgress, ReconciledOutcome, RemovalProgress,
    RemoveDeployment, ResourceAction, ResourceEffect,
};

/// Mutable progress, separate from the immutable resource plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceProgress {
    /// Installation progress for this exact resource.
    pub install: InstallProgress,
    /// Removal progress; external resources remain reference-only.
    pub removal: RemovalProgress,
    /// Monotonic compare-and-swap version.
    pub version: u64,
    /// Current durable attempt requiring completion or reconciliation.
    pub active_attempt: Option<DeploymentAttemptId>,
    /// Bounded safe diagnostic, without raw provider text.
    pub diagnostic: Option<DiagnosticCode>,
}

/// Safe inspectable durable state without provider handles, host paths, or credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentSnapshot {
    /// Revalidated immutable declaration and resolved intent.
    pub intent: DeploymentIntent,
    /// Deployment lifecycle independent of each resource's progress.
    pub lifecycle: DeploymentLifecycle,
    /// Monotonic deployment compare-and-swap version.
    pub version: u64,
    /// Exact named progress, including retained and external resources.
    pub resources: BTreeMap<CapabilitySlotKey, ResourceProgress>,
}

/// Admission distinguishes fresh durable intent from ongoing work and completed replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionDisposition {
    /// New intent or removal command was durably admitted.
    Created,
    /// Unfinished unchanged input requires resumable work or reconciliation.
    Resume,
    /// Completed unchanged input returns its original result after live authorization.
    Replay,
}

/// Committed admission result; no provider action is part of repository admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeploymentAdmission {
    /// Whether the command is new, unfinished, or a completed replay.
    pub disposition: AdmissionDisposition,
    /// Current authorized deployment state.
    pub snapshot: DeploymentSnapshot,
    /// Original committed admission receipt, stable on replay.
    pub receipt: CommandReceipt,
}

/// Append-only safe attempt projection for adapter audit history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttemptRecord {
    /// Exact request and actor provenance.
    pub provenance: AttemptProvenance,
    /// Previously admitted logical command.
    pub command: CommandIdentity,
    /// Exact named resource.
    pub resource: CapabilitySlotKey,
    /// Action whose provider result is recorded.
    pub action: ResourceAction,
    /// Fencing generation, monotonically increasing per resource.
    pub generation: u64,
    /// Safe failure or ambiguity code, if present.
    pub diagnostic: Option<DiagnosticCode>,
}

/// Authoritative durable deployment boundary, independent of provider execution.
///
/// Implementations reload authoritative catalog and external-resource facts; input
/// planning conveys no grants. Open an actor transaction and check project/resource
/// permissions before *every* admission, resumed effect, inspect, and replay. Audit
/// command decisions and commit progress, receipt, and product outbox events together.
/// Compare immutable input before replay, including project-local deployment-key
/// conflicts. Retained ownership and tombstones remain discoverable.
///
/// Claim commits before provider work; transactions never span provider calls. CAS
/// checks the exact attempt, version, and generation. A crash, cancellation, or
/// expired execution claim requires reconciliation; none proves provider absence
/// or detached storage. Provider handles stay inside their owning adapters.
#[async_trait]
pub trait DeploymentRepository: Send + Sync + 'static {
    /// Admits or resumes matching intent after current source and target authorization.
    ///
    /// # Errors
    /// Rejects live denial, changed immutable input, or an unavailable repository.
    async fn admit_install(
        &self,
        identity: &AuthenticatedIdentity,
        command: InstallDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError>;

    /// Reads authorized safe state, including partial failures and retained ownership.
    ///
    /// # Errors
    /// Rejects live denial, unavailable records, or invalid persisted intent.
    async fn inspect(
        &self,
        identity: &AuthenticatedIdentity,
        deployment_id: DeploymentId,
    ) -> Result<DeploymentSnapshot, DeploymentError>;

    /// Admits policy-bounded removal; replay authorization precedes CAS/input lookup.
    ///
    /// # Errors
    /// Rejects live denial, changed command input, or a stale deployment version.
    async fn admit_remove(
        &self,
        identity: &AuthenticatedIdentity,
        command: RemoveDeployment,
    ) -> Result<DeploymentAdmission, DeploymentError>;

    /// Admits a new current manager command while preserving old removal work.
    ///
    /// Requires explicit configured support, current ALL-owned cleanup rights and
    /// exact immutable old claims/closures. Removed terminal inspection is separate.
    ///
    /// # Errors
    /// Rejects unsupported adapters, missing history, denied access or stale CAS.
    async fn admit_remove_resume(
        &self,
        _identity: &AuthenticatedIdentity,
        _command: RemoveDeployment,
    ) -> Result<crate::RemoveResumeAdmission, DeploymentError> {
        Err(DeploymentError::InvalidAction)
    }

    /// Commits a fresh authorized CAS claim before any resource side effect.
    ///
    /// Rejects dependency/state conflicts and all external mutation; ambiguous
    /// outcomes must be reconciled rather than issuing another creation.
    ///
    /// # Errors
    /// Rejects live denial, stale versions, unsupported actions, or unresolved claims.
    async fn claim_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        effect: ResourceEffect,
    ) -> Result<EffectClaim, DeploymentError>;

    /// Records a definite exact-identity outcome with claim/generation CAS.
    ///
    /// # Errors
    /// Rejects live denial, stale claims, mismatched evidence, or repository failure.
    async fn complete_resource_effect(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        evidence: EffectEvidence,
    ) -> Result<ResourceProgress, DeploymentError>;

    /// Preserves unknown provider outcome and blocks further effects pending proof.
    ///
    /// # Errors
    /// Rejects live denial, stale claims, or repository failure.
    async fn mark_resource_ambiguous(
        &self,
        identity: &AuthenticatedIdentity,
        claim: &EffectClaim,
        diagnostic: DiagnosticCode,
    ) -> Result<ResourceProgress, DeploymentError>;

    /// Records a fresh authorized reconciliation against an immutable target claim.
    ///
    /// `command` belongs to the current actor. An admitted removal may reconcile
    /// an earlier owned installation claim without borrowing its actor's command
    /// or requiring continued source access. It never reopens installation.
    ///
    /// NotApplied requires authoritative absence proof. Delete/detach completion
    /// requires proof of drain, detach, and fencing; unproven results stay blocked.
    ///
    /// # Errors
    /// Rejects live denial, stale claims, insufficient evidence, or repository failure.
    async fn record_reconciliation(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        claim: &EffectClaim,
        provenance: AttemptProvenance,
        outcome: ReconciledOutcome,
    ) -> Result<ResourceProgress, DeploymentError>;

    /// Completes only after all resource states satisfy install or safe removal policy.
    ///
    /// Rechecks authority before returning an unchanged completed receipt. External
    /// resources never become deleted, and retained records never lose provenance.
    ///
    /// # Errors
    /// Rejects live denial, changed inputs, stale versions, or incomplete resources.
    async fn finish_command(
        &self,
        identity: &AuthenticatedIdentity,
        command: CommandIdentity,
        deployment_id: DeploymentId,
        expected_version: u64,
    ) -> Result<CommandReceipt, DeploymentError>;
}
