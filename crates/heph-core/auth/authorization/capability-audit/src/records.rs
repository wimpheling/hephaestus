use super::{
    CapabilityAuditError, CapabilityAuditEventKind, CapabilityAuditPage, CapabilityAuditReason,
    CapabilityDecision, CapabilityUseOutcome, NewCapabilityAuditEvent,
};
use async_trait::async_trait;
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, CapabilityResource,
    CapabilitySlotKey, RuntimeSessionId,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use time::OffsetDateTime;
use uuid::Uuid;

/// Redacted capability evidence visible to an authorized run inspector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAuditRecord {
    /// Stable event identifier.
    pub id: Uuid,
    /// Run whose exact runtime emitted the event.
    pub run_id: RunId,
    /// Durable workload principal.
    pub instance_id: AgentInstanceId,
    /// Immutable workload revision.
    pub instance_revision_id: AgentInstanceRevisionId,
    /// Exact runtime session.
    pub runtime_session_id: RuntimeSessionId,
    /// Immutable authorization snapshot.
    pub snapshot_id: AuthorizationSnapshotId,
    /// Exact capability binding.
    pub binding_id: CapabilityBindingId,
    /// Symbolic release slot.
    pub slot: CapabilitySlotKey,
    /// Exact opaque resource and kind.
    pub resource: CapabilityResource,
    /// User who created the immutable capability binding.
    pub grantor_id: UserId,
    /// Semantic operation attempted.
    pub operation: CapabilityOperation,
    /// Request correlation identifier.
    pub request_id: RequestId,
    /// Evidence classification.
    pub kind: CapabilityAuditEventKind,
    /// Decision for authorization evidence.
    pub decision: Option<CapabilityDecision>,
    /// Outcome for capability-use evidence.
    pub outcome: Option<CapabilityUseOutcome>,
    /// Optional safe machine reason.
    pub reason: Option<CapabilityAuditReason>,
    /// Authorization model used for the decision.
    pub authorization_model_version: String,
    /// Time the event occurred.
    pub occurred_at: OffsetDateTime,
}

/// Persistence and authorized inspection port for capability evidence.
#[async_trait]
pub trait CapabilityAuditRepository: Send + Sync {
    /// Appends one immutable event as a trusted runtime worker.
    async fn append(&self, event: &NewCapabilityAuditEvent) -> Result<(), CapabilityAuditError>;

    /// Lists a run's redacted events for a currently authorized user.
    async fn list_for_run(
        &self,
        identity: &AuthenticatedIdentity,
        run_id: RunId,
        page: CapabilityAuditPage,
    ) -> Result<Vec<CapabilityAuditRecord>, CapabilityAuditError>;
}
