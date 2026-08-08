//! Redacted capability audit recording and authorized inspection contracts.
//!
//! This boundary deliberately accepts only opaque identifiers, closed enums,
//! and bounded machine reason codes. It cannot receive request payloads,
//! credentials, paths, secret values, or provider responses.

use async_trait::async_trait;
use capability_domain::{
    AuthorizationSnapshotId, CapabilityBindingId, CapabilityOperation, CapabilityResource,
    CapabilitySlotKey, RuntimeSessionId,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, RunId};
use std::{error::Error, fmt};
use time::OffsetDateTime;
use uuid::Uuid;

/// Maximum number of events returned by one inspection request.
pub const MAX_AUDIT_PAGE_SIZE: u16 = 200;

/// Kind of immutable capability evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityAuditEventKind {
    /// The runtime request was evaluated against immutable and live authority.
    AuthorizationDecision,
    /// An allowed capability invocation completed or failed.
    CapabilityUse,
}

impl CapabilityAuditEventKind {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AuthorizationDecision => "authorization_decision",
            Self::CapabilityUse => "capability_use",
        }
    }
}

/// Result of one capability authorization evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityDecision {
    /// The request is allowed to proceed.
    Allow,
    /// The request must not invoke the capability.
    Deny,
}

impl CapabilityDecision {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        }
    }
}

/// Outcome of an already-authorized capability invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityUseOutcome {
    /// The controlled operation completed successfully.
    Succeeded,
    /// The controlled operation failed after authorization.
    Failed,
}

impl CapabilityUseOutcome {
    /// Returns the stable persistence representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
        }
    }
}

/// A bounded, non-sensitive machine reason suitable for durable audit rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityAuditReason(String);

impl CapabilityAuditReason {
    /// Parses a lower-snake-case reason code of at most 64 bytes.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, or non-canonical values.
    pub fn parse(value: impl Into<String>) -> Result<Self, CapabilityAuditError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let Some(first) = bytes.next() else {
            return Err(CapabilityAuditError::InvalidReasonCode);
        };
        if value.len() > 64
            || !first.is_ascii_lowercase()
            || bytes
                .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_'))
        {
            return Err(CapabilityAuditError::InvalidReasonCode);
        }
        Ok(Self(value))
    }

    /// Returns the canonical reason code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

/// Provider-neutral capability audit failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CapabilityAuditError {
    /// A reason code was not canonical lower snake case.
    #[error("capability audit reason code is invalid")]
    InvalidReasonCode,
    /// The requested page size was outside the supported bound.
    #[error("capability audit page size is invalid")]
    InvalidPageSize,
    /// Persisted evidence was malformed or inconsistent.
    #[error("capability audit evidence is invalid")]
    InvalidEvidence,
    /// The caller cannot inspect the run, or the run is deliberately hidden.
    #[error("capability audit evidence is unavailable")]
    Unavailable,
    /// The configured provider failed.
    #[error("capability audit provider failed: {0}")]
    Provider(#[source] Box<dyn Error + Send + Sync>),
}

impl CapabilityAuditError {
    /// Wraps a provider failure without exposing its concrete type.
    #[must_use]
    pub fn provider(error: impl Error + Send + Sync + 'static) -> Self {
        Self::Provider(Box::new(error))
    }
}

impl PartialEq for CapabilityAuditError {
    fn eq(&self, other: &Self) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

impl Eq for CapabilityAuditError {}

impl fmt::Display for CapabilityAuditReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{CapabilityAuditError, CapabilityAuditPage, CapabilityAuditReason};

    #[test]
    fn reason_codes_are_bounded_machine_values() {
        assert_eq!(
            CapabilityAuditReason::parse("live_authorization_revoked")
                .expect("canonical reason")
                .as_str(),
            "live_authorization_revoked"
        );
        for invalid in ["", "ContainsSecret", "has-hyphen", "has space"] {
            assert_eq!(
                CapabilityAuditReason::parse(invalid),
                Err(CapabilityAuditError::InvalidReasonCode)
            );
        }
        assert_eq!(
            CapabilityAuditReason::parse(format!("a{}", "b".repeat(64))),
            Err(CapabilityAuditError::InvalidReasonCode)
        );
    }

    #[test]
    fn inspection_pages_are_bounded() {
        assert_eq!(
            CapabilityAuditPage::new(0, None),
            Err(CapabilityAuditError::InvalidPageSize)
        );
        assert!(CapabilityAuditPage::new(200, None).is_ok());
        assert_eq!(
            CapabilityAuditPage::new(201, None),
            Err(CapabilityAuditError::InvalidPageSize)
        );
    }
}
