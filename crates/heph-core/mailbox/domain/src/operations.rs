use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{
    DeduplicationKey, MailboxEventId, MailboxId, MailboxOperationId, OPERATION_ID_DOMAIN,
    ProducerId,
};

/// Operation kind used to deterministically derive an idempotency identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailboxOperationKind {
    /// Accept one producer-scoped event publication.
    Publish,
    /// Claim an eligible event for dispatch.
    Dispatch,
    /// Start one logical application delivery attempt.
    Attempt,
    /// Schedule one later logical delivery attempt.
    Retry,
    /// Stop a live or pending event by authorization.
    Cancel,
    /// Retain one event as terminally undeliverable.
    DeadLetter,
}

impl MailboxOperationKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Dispatch => "dispatch",
            Self::Attempt => "attempt",
            Self::Retry => "retry",
            Self::Cancel => "cancel",
            Self::DeadLetter => "dead_letter",
        }
    }
}

/// Exact input from which a deterministic mailbox operation ID is derived.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxOperationIdentity {
    /// Operation purpose.
    pub kind: MailboxOperationKind,
    /// Mailbox scope.
    pub mailbox_id: MailboxId,
    /// Event scope, when the event has already been accepted.
    pub event_id: Option<MailboxEventId>,
    /// Producer identity for publish deduplication.
    pub producer_id: Option<ProducerId>,
    /// Producer key for publish deduplication.
    pub deduplication_key: Option<DeduplicationKey>,
    /// Logical attempt number for attempt-derived operations.
    pub attempt_number: Option<u32>,
}

impl MailboxOperationIdentity {
    /// Creates the identity for one producer-scoped publish.
    #[must_use]
    pub const fn publish(
        mailbox_id: MailboxId,
        producer_id: ProducerId,
        deduplication_key: DeduplicationKey,
    ) -> Self {
        Self {
            kind: MailboxOperationKind::Publish,
            mailbox_id,
            event_id: None,
            producer_id: Some(producer_id),
            deduplication_key: Some(deduplication_key),
            attempt_number: None,
        }
    }
    /// Creates the identity for one event dispatch transition.
    ///
    /// Each logical attempt needs a distinct durable dispatch wake-up. The
    /// number is one-based so a retry cannot collide with the already
    /// published dispatch command for an earlier attempt.
    #[must_use]
    pub const fn dispatch(
        mailbox_id: MailboxId,
        event_id: MailboxEventId,
        attempt_number: u32,
    ) -> Self {
        Self {
            kind: MailboxOperationKind::Dispatch,
            mailbox_id,
            event_id: Some(event_id),
            producer_id: None,
            deduplication_key: None,
            attempt_number: Some(attempt_number),
        }
    }
    /// Creates the identity for one logical attempt.
    #[must_use]
    pub const fn attempt(
        mailbox_id: MailboxId,
        event_id: MailboxEventId,
        attempt_number: u32,
    ) -> Self {
        Self {
            kind: MailboxOperationKind::Attempt,
            mailbox_id,
            event_id: Some(event_id),
            producer_id: None,
            deduplication_key: None,
            attempt_number: Some(attempt_number),
        }
    }
    /// Creates the identity for one retry transition.
    #[must_use]
    pub const fn retry(
        mailbox_id: MailboxId,
        event_id: MailboxEventId,
        attempt_number: u32,
    ) -> Self {
        Self {
            kind: MailboxOperationKind::Retry,
            mailbox_id,
            event_id: Some(event_id),
            producer_id: None,
            deduplication_key: None,
            attempt_number: Some(attempt_number),
        }
    }
    /// Creates the identity for one cancellation transition.
    #[must_use]
    pub const fn cancellation(mailbox_id: MailboxId, event_id: MailboxEventId) -> Self {
        Self {
            kind: MailboxOperationKind::Cancel,
            mailbox_id,
            event_id: Some(event_id),
            producer_id: None,
            deduplication_key: None,
            attempt_number: None,
        }
    }
    /// Creates the identity for one dead-letter transition.
    #[must_use]
    pub const fn dead_letter(
        mailbox_id: MailboxId,
        event_id: MailboxEventId,
        attempt_number: u32,
    ) -> Self {
        Self {
            kind: MailboxOperationKind::DeadLetter,
            mailbox_id,
            event_id: Some(event_id),
            producer_id: None,
            deduplication_key: None,
            attempt_number: Some(attempt_number),
        }
    }

    /// Derives the stable idempotency ID from canonical operation inputs.
    #[must_use]
    pub fn id(&self) -> MailboxOperationId {
        let mut bytes = Vec::with_capacity(256);
        bytes.extend_from_slice(OPERATION_ID_DOMAIN);
        bytes.extend_from_slice(self.kind.as_str().as_bytes());
        bytes.push(0);
        bytes.extend_from_slice(self.mailbox_id.as_uuid().as_bytes());
        push_uuid(&mut bytes, self.event_id);
        push_string(
            &mut bytes,
            self.producer_id.as_ref().map(ProducerId::as_str),
        );
        push_string(
            &mut bytes,
            self.deduplication_key
                .as_ref()
                .map(DeduplicationKey::as_str),
        );
        bytes.extend_from_slice(&self.attempt_number.unwrap_or_default().to_be_bytes());
        let digest = Sha256::digest(bytes);
        let mut identifier = [0_u8; 16];
        identifier.copy_from_slice(&digest[..16]);
        identifier[6] = (identifier[6] & 0x0f) | 0x80;
        identifier[8] = (identifier[8] & 0x3f) | 0x80;
        MailboxOperationId::from_uuid(Uuid::from_bytes(identifier))
    }
}
fn push_uuid(bytes: &mut Vec<u8>, value: Option<MailboxEventId>) {
    match value {
        Some(value) => bytes.extend_from_slice(value.as_uuid().as_bytes()),
        None => bytes.extend_from_slice(&[0_u8; 16]),
    }
}

fn push_string(bytes: &mut Vec<u8>, value: Option<&str>) {
    match value {
        Some(value) => {
            let length = u32::try_from(value.len()).expect("bounded mailbox value length");
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(value.as_bytes());
        }
        None => bytes.extend_from_slice(&u32::MAX.to_be_bytes()),
    }
}
