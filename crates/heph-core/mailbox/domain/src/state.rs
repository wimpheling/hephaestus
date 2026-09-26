use serde::{Deserialize, Serialize};

use super::{
    AgentInstanceId, DeduplicationKey, DeliveryAttemptId, MailboxEnvelope, MailboxEventId,
    MailboxId, ProducerId, errors::MailboxDomainError,
};

/// Lifecycle of a durable mailbox event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    /// Accepted durably but not yet eligible to start.
    Pending,
    /// Rechecked and eligible for a dispatch claim.
    Eligible,
    /// Claimed durably by one dispatcher.
    Leased,
    /// A guest execution has begun for the exact claim.
    Running,
    /// The application result committed successfully.
    Delivered,
    /// A new logical attempt may be scheduled after its retry time.
    Retryable,
    /// Current authority or lifecycle denied delivery.
    Denied,
    /// Terminally retained without successful application delivery.
    DeadLettered,
    /// Explicitly stopped without successful application delivery.
    Cancelled,
}

impl DeliveryState {
    /// Returns whether a transition is valid under the mailbox state machine.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        use DeliveryState::{
            Cancelled, DeadLettered, Delivered, Denied, Eligible, Leased, Pending, Retryable,
            Running,
        };
        matches!(
            (self, next),
            (Pending, Eligible | Denied | Cancelled)
                | (Eligible, Leased | Denied | Cancelled)
                | (Leased, Running | Retryable | Denied | Cancelled)
                | (
                    Running,
                    Delivered | Retryable | Denied | DeadLettered | Cancelled
                )
                | (Retryable, Eligible | DeadLettered | Cancelled)
                | (Denied, Eligible | Cancelled)
        )
    }

    /// Returns whether no later normal delivery may begin.
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Delivered | Self::DeadLettered | Self::Cancelled)
    }
}

/// A terminal or current mailbox-event disposition retained for inspection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryDisposition {
    /// The event remains pending lifecycle/authorization eligibility.
    Pending,
    /// The event is currently claimed or executing.
    InProgress,
    /// The application result committed successfully.
    Delivered,
    /// Authorization or lifecycle rejected delivery.
    Denied,
    /// Retry remains scheduled.
    RetryScheduled,
    /// Retry policy terminally retained the event.
    DeadLettered,
    /// An authorized cancellation stopped the event.
    Cancelled,
}

impl From<DeliveryState> for DeliveryDisposition {
    fn from(value: DeliveryState) -> Self {
        match value {
            DeliveryState::Pending | DeliveryState::Eligible => Self::Pending,
            DeliveryState::Leased | DeliveryState::Running => Self::InProgress,
            DeliveryState::Delivered => Self::Delivered,
            DeliveryState::Retryable => Self::RetryScheduled,
            DeliveryState::Denied => Self::Denied,
            DeliveryState::DeadLettered => Self::DeadLettered,
            DeliveryState::Cancelled => Self::Cancelled,
        }
    }
}

/// A monotonic, non-zero dispatch sequence scoped to one agent instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "u64", into = "u64")]
pub struct DispatchSequence(u64);

impl DispatchSequence {
    /// Creates a non-zero sequence.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidDispatchSequence`] for zero.
    pub const fn new(value: u64) -> Result<Self, MailboxDomainError> {
        if value == 0 {
            return Err(MailboxDomainError::InvalidDispatchSequence);
        }
        Ok(Self(value))
    }

    /// Returns the underlying sequence number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl TryFrom<u64> for DispatchSequence {
    type Error = MailboxDomainError;
    fn try_from(value: u64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}
impl From<DispatchSequence> for u64 {
    fn from(value: DispatchSequence) -> Self {
        value.0
    }
}

/// Durable state-volume access outcome without claims about application bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StateAccessOutcome {
    /// The attempt did not require a persistent state volume.
    NoState,
    /// The state volume was accessed and guest teardown completed.
    CompletedAccess,
    /// State access or required cleanup failed.
    FailedAccess,
    /// A crash or lost evidence prevents a reliable state-access conclusion.
    UncertainAccess,
}

/// Immutable accepted event metadata; its body remains an opaque reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEvent {
    /// Stable accepted-event identity.
    pub id: MailboxEventId,
    /// Mailbox that owns the event.
    pub mailbox_id: MailboxId,
    /// Owning agent instance.
    pub instance_id: AgentInstanceId,
    /// Producer identity declared at acceptance.
    pub producer_id: ProducerId,
    /// Producer-scoped stable deduplication key.
    pub deduplication_key: DeduplicationKey,
    /// Bounded opaque delivery envelope.
    pub envelope: MailboxEnvelope,
}

/// One logical mailbox delivery attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAttempt {
    /// Stable attempt identity.
    pub id: DeliveryAttemptId,
    /// Event whose processing this attempt represents.
    pub event_id: MailboxEventId,
    /// One-based logical attempt number owned by `PostgreSQL`.
    pub number: u32,
    /// Monotonic instance-local order for stateful dispatch.
    pub dispatch_sequence: DispatchSequence,
    /// Current state-access conclusion.
    pub state_access_outcome: StateAccessOutcome,
}

impl DeliveryAttempt {
    /// Creates a logical attempt with a one-based attempt number.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidAttemptNumber`] for zero.
    pub const fn new(
        id: DeliveryAttemptId,
        event_id: MailboxEventId,
        number: u32,
        dispatch_sequence: DispatchSequence,
        state_access_outcome: StateAccessOutcome,
    ) -> Result<Self, MailboxDomainError> {
        if number == 0 {
            return Err(MailboxDomainError::InvalidAttemptNumber);
        }
        Ok(Self {
            id,
            event_id,
            number,
            dispatch_sequence,
            state_access_outcome,
        })
    }
}
