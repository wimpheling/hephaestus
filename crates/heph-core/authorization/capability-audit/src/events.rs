use super::{
    CapabilityAuditError, CapabilityAuditEventKind, CapabilityAuditReason, CapabilityDecision,
    CapabilityUseOutcome, MAX_AUDIT_PAGE_SIZE,
};
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, RuntimeSessionId,
};
use identity_domain::RequestId;
use time::OffsetDateTime;
use uuid::Uuid;

/// Exact immutable authority references common to decision and use evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityAuditContext {
    /// Exact short-lived runtime session.
    pub runtime_session_id: RuntimeSessionId,
    /// Immutable dispatch-time authorization snapshot.
    pub snapshot_id: AuthorizationSnapshotId,
    /// Exact binding within the snapshot.
    pub binding_id: CapabilityBindingId,
    /// Semantic operation being attempted.
    pub operation: CapabilityOperation,
    /// Per-request correlation identifier.
    pub request_id: RequestId,
    /// Authorization model used for the live decision.
    pub authorization_model_version: &'static str,
}

/// One immutable event to append to the capability audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCapabilityAuditEvent {
    /// Stable event identifier.
    pub id: Uuid,
    /// Exact authority references.
    pub context: CapabilityAuditContext,
    /// Event classification.
    pub kind: CapabilityAuditEventKind,
    /// Authorization decision, present only for decision evidence.
    pub decision: Option<CapabilityDecision>,
    /// Invocation outcome, present only for use evidence.
    pub outcome: Option<CapabilityUseOutcome>,
    /// Optional non-sensitive machine reason.
    pub reason: Option<CapabilityAuditReason>,
    /// Time the decision or outcome occurred.
    pub occurred_at: OffsetDateTime,
}

impl NewCapabilityAuditEvent {
    /// Creates authorization decision evidence.
    #[must_use]
    pub fn decision(
        context: CapabilityAuditContext,
        decision: CapabilityDecision,
        reason: Option<CapabilityAuditReason>,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            context,
            kind: CapabilityAuditEventKind::AuthorizationDecision,
            decision: Some(decision),
            outcome: None,
            reason,
            occurred_at,
        }
    }

    /// Creates use outcome evidence after an allowed decision.
    #[must_use]
    pub fn capability_use(
        context: CapabilityAuditContext,
        outcome: CapabilityUseOutcome,
        reason: Option<CapabilityAuditReason>,
        occurred_at: OffsetDateTime,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            context,
            kind: CapabilityAuditEventKind::CapabilityUse,
            decision: None,
            outcome: Some(outcome),
            reason,
            occurred_at,
        }
    }
}

/// Opaque cursor for stable reverse-chronological inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityAuditCursor {
    /// Event occurrence time.
    pub occurred_at: OffsetDateTime,
    /// Event identifier used as the deterministic tie-breaker.
    pub id: Uuid,
}

/// Validated bounded inspection request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityAuditPage {
    limit: u16,
    before: Option<CapabilityAuditCursor>,
}

impl CapabilityAuditPage {
    /// Creates a bounded page request.
    ///
    /// # Errors
    ///
    /// Rejects zero or more than [`MAX_AUDIT_PAGE_SIZE`] rows.
    pub const fn new(
        limit: u16,
        before: Option<CapabilityAuditCursor>,
    ) -> Result<Self, CapabilityAuditError> {
        if limit == 0 || limit > MAX_AUDIT_PAGE_SIZE {
            return Err(CapabilityAuditError::InvalidPageSize);
        }
        Ok(Self { limit, before })
    }

    /// Returns the requested row limit.
    #[must_use]
    pub const fn limit(self) -> u16 {
        self.limit
    }

    /// Returns the optional exclusive cursor.
    #[must_use]
    pub const fn before(self) -> Option<CapabilityAuditCursor> {
        self.before
    }
}
