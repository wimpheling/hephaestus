use super::parsing::valid_observed_repository_path;
use super::prelude::*;
/// A new authenticated Zot notification after transport validation.
#[derive(Debug, Clone)]
pub struct NewRegistryNotification {
    /// Stable upstream event key.
    pub event_key: String,
    /// Canonical repository path observed by Zot.
    pub repository_path: String,
    /// Distribution operation observed by Zot.
    pub action: RegistryNotificationAction,
    /// Optional target descriptor, when the notification includes one.
    pub target: Option<RegistryNotificationTarget>,
    /// Upstream occurrence timestamp.
    pub occurred_at: OffsetDateTime,
    /// SHA-256 of the exact transport body, never the retained body itself.
    pub payload_sha256: [u8; 32],
}

/// Digest and media type observed in a Zot event. Descriptor size is resolved
/// authoritatively from Zot during reconciliation, not trusted from callbacks.
#[derive(Debug, Clone)]
pub struct RegistryNotificationTarget {
    /// Observed manifest digest.
    pub digest: Sha256Digest,
    /// Observed manifest media type.
    pub media_type: OciMediaType,
}

impl NewRegistryNotification {
    pub(super) fn validate(&self) -> Result<(), RegistryStoreError> {
        ((1..=200).contains(&self.event_key.len())
            && self.event_key == self.event_key.trim()
            && !self.event_key.bytes().any(|byte| byte.is_ascii_control())
            && valid_observed_repository_path(&self.repository_path))
        .then_some(())
        .ok_or(RegistryStoreError::InvalidNotification)
    }
}

/// Bounded Zot notification action retained by the durable inbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryNotificationAction {
    /// Manifest or artifact push observation.
    Push,
    /// Pull observation.
    Pull,
    /// Delete observation.
    Delete,
}

impl RegistryNotificationAction {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Push => "push",
            Self::Pull => "pull",
            Self::Delete => "delete",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self, RegistryStoreError> {
        match value {
            "push" => Ok(Self::Push),
            "pull" => Ok(Self::Pull),
            "delete" => Ok(Self::Delete),
            _ => Err(RegistryStoreError::InvalidStoredData),
        }
    }
}

/// Durable notification receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryNotificationReceipt {
    /// Durable inbox identity.
    pub id: Uuid,
    /// Idempotency key.
    pub event_key: String,
    /// Whether the exact event was already durable.
    pub duplicate: bool,
}

/// A notification exclusively claimed by one reducer lease.
#[derive(Debug, Clone)]
pub struct ClaimedRegistryNotification {
    /// Durable inbox identity.
    pub id: Uuid,
    /// Lease capability required for completion.
    pub claim_token: Uuid,
    /// Canonical namespace path.
    pub repository_path: String,
    /// Parsed forge-owned namespace, absent for bounded orphan observations.
    pub namespace: Option<RegistryNamespace>,
    /// Observed action.
    pub action: RegistryNotificationAction,
    /// Optional target descriptor.
    pub target: Option<RegistryNotificationTarget>,
    /// Upstream event timestamp.
    pub occurred_at: OffsetDateTime,
}

/// Terminal reduction result for an inbox observation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationCompletion {
    /// The observation has been reduced or queued for reconciliation.
    Processed,
    /// The observation is permanently invalid, with a bounded safe code.
    Rejected {
        /// Stable non-sensitive rejection code.
        failure_code: String,
    },
}

/// Registry adapter failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RegistryStoreError {
    /// The caller supplied data that violates domain validation.
    #[error("registry value is invalid: {0}")]
    InvalidValue(#[from] RegistryValueError),
    /// A state edge or verification conflicted with the durable lifecycle.
    #[error("registry lifecycle transition is invalid: {0}")]
    Lifecycle(#[source] PublicationLifecycleError),
    /// A stored row cannot be reconstructed as a strict domain value.
    #[error("registry storage contains invalid control-plane data")]
    InvalidStoredData,
    /// An operation lost an ownership, idempotency, or lease race.
    #[error("registry control-plane operation conflicted")]
    Conflict,
    /// A notification failed local bounded-field validation.
    #[error("registry notification is invalid")]
    InvalidNotification,
    /// `PostgreSQL` failed.
    #[error("registry storage failed")]
    Storage(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Clone, Copy)]
pub(super) enum Transition {
    Begin,
    Retry,
    Approve,
    Missing,
    Retire,
}

impl Transition {
    pub(super) fn apply(
        self,
        intent: PublicationIntent,
    ) -> Result<PublicationIntent, RegistryStoreError> {
        match self {
            Self::Begin => intent.begin_publishing(),
            Self::Retry => intent.retry(),
            Self::Approve => intent.approve(),
            Self::Missing => intent.mark_missing(),
            Self::Retire => Ok(intent.retire()),
        }
        .map_err(RegistryStoreError::Lifecycle)
    }
}
