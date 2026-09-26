//! Provider-neutral durable mailbox envelopes, delivery state, and identities.
//!
//! Mailbox payloads remain opaque to the control plane. This crate validates
//! their bounded metadata and defines the logical state machine; `PostgreSQL`,
//! NATS, and VM adapters implement durable storage and execution separately.

use runtime_types::AgentInstanceId;

/// Maximum UTF-8 byte length of a mailbox method.
pub const MAX_METHOD_BYTES: usize = 16;
/// Maximum UTF-8 byte length of a mailbox route.
pub const MAX_ROUTE_BYTES: usize = 1024;
/// Maximum selected headers in one envelope.
pub const MAX_HEADERS: usize = 32;
/// Maximum UTF-8 byte length of a header name.
pub const MAX_HEADER_NAME_BYTES: usize = 64;
/// Maximum UTF-8 byte length of one selected header value.
pub const MAX_HEADER_VALUE_BYTES: usize = 1024;
/// Maximum UTF-8 byte length of content metadata.
pub const MAX_CONTENT_METADATA_BYTES: usize = 256;
/// Maximum UTF-8 byte length of a trace context.
pub const MAX_TRACE_CONTEXT_BYTES: usize = 512;
/// Maximum UTF-8 byte length of an opaque producer key.
pub const MAX_PRODUCER_KEY_BYTES: usize = 128;
/// Maximum UTF-8 byte length of a deduplication key.
pub const MAX_DEDUPLICATION_KEY_BYTES: usize = 256;
/// Maximum accepted payload bytes for one mailbox body.
pub const MAX_BODY_BYTES: u32 = 1_048_576;
/// Length of a SHA-256 payload integrity hash.
pub const BODY_INTEGRITY_HASH_BYTES: usize = 32;

const OPERATION_ID_DOMAIN: &[u8] = b"hephaestus.mailbox-operation.v1\0";

mod envelope;
mod errors;
mod identifiers;
mod operations;
mod routing;
mod state;

pub use envelope::{BodyReference, ContentMetadata, MailboxEnvelope};
pub use errors::MailboxDomainError;
pub use identifiers::{
    BodyReferenceId, DeduplicationKey, DeliveryAttemptId, EnvelopeMethod, MailboxEventId,
    MailboxId, MailboxOperationId, ProducerId,
};
pub use operations::{MailboxOperationIdentity, MailboxOperationKind};
pub use routing::{EnvelopeRoute, SelectedHeaderName, SelectedHeaderValue, TraceContext};
pub use state::{
    DeliveryAttempt, DeliveryDisposition, DeliveryState, DispatchSequence, MailboxEvent,
    StateAccessOutcome,
};

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use serde_json::json;
    use time::OffsetDateTime;
    use uuid::Uuid;

    fn body() -> BodyReference {
        BodyReference::new(
            BodyReferenceId::from_uuid(Uuid::nil()),
            0,
            [0; BODY_INTEGRITY_HASH_BYTES],
        )
        .expect("body")
    }

    #[test]
    fn accepts_bounded_provider_neutral_envelope() {
        let headers = BTreeMap::from([(
            SelectedHeaderName::parse("x-request-id").expect("name"),
            SelectedHeaderValue::parse("abc").expect("value"),
        )]);
        let envelope = MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/hooks/example?value=1").expect("route"),
            headers,
            ContentMetadata::new(body(), Some("application/json".into()), None).expect("content"),
            OffsetDateTime::UNIX_EPOCH,
            Some(TraceContext::parse("00-abc-def-01").expect("trace")),
        )
        .expect("envelope");
        let encoded = serde_json::to_value(&envelope).expect("serialize");
        assert_eq!(encoded["method"], json!("POST"));
        assert_eq!(encoded["content"]["body"]["byte_length"], json!(0));
    }

    #[test]
    fn rejects_malformed_envelope_values_and_bounds() {
        assert!(EnvelopeMethod::parse("post").is_err());
        assert!(EnvelopeRoute::parse("https://example.test/hook").is_err());
        assert!(EnvelopeRoute::parse("//authority").is_err());
        assert!(SelectedHeaderName::parse("X-Test").is_err());
        assert!(SelectedHeaderValue::parse("ok\r\nnext").is_err());
        assert!(TraceContext::parse(" ").is_err());
        assert!(
            BodyReference::new(
                BodyReferenceId::new(),
                MAX_BODY_BYTES + 1,
                [0; BODY_INTEGRITY_HASH_BYTES]
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_more_than_bounded_selected_headers() {
        let headers = (0..=MAX_HEADERS)
            .map(|number| {
                (
                    SelectedHeaderName::parse(format!("x-{number}")).expect("name"),
                    SelectedHeaderValue::parse("v").expect("value"),
                )
            })
            .collect();
        assert_eq!(
            MailboxEnvelope::new(
                EnvelopeMethod::parse("POST").expect("method"),
                EnvelopeRoute::parse("/").expect("route"),
                headers,
                ContentMetadata::new(body(), None, None).expect("content"),
                OffsetDateTime::UNIX_EPOCH,
                None
            ),
            Err(MailboxDomainError::TooManyHeaders)
        );
    }

    #[test]
    fn lifecycle_transitions_are_deliberate() {
        assert!(DeliveryState::Pending.can_transition_to(DeliveryState::Eligible));
        assert!(DeliveryState::Eligible.can_transition_to(DeliveryState::Leased));
        assert!(DeliveryState::Leased.can_transition_to(DeliveryState::Running));
        assert!(DeliveryState::Running.can_transition_to(DeliveryState::Retryable));
        assert!(DeliveryState::Retryable.can_transition_to(DeliveryState::Eligible));
        assert!(DeliveryState::Running.can_transition_to(DeliveryState::Delivered));
        assert!(!DeliveryState::Delivered.can_transition_to(DeliveryState::Eligible));
        assert!(!DeliveryState::Pending.can_transition_to(DeliveryState::Running));
        assert!(DeliveryState::DeadLettered.is_terminal());
        assert_eq!(
            DeliveryDisposition::from(DeliveryState::Retryable),
            DeliveryDisposition::RetryScheduled
        );
    }

    #[test]
    fn attempts_and_sequences_reject_zero() {
        assert_eq!(
            DispatchSequence::new(0),
            Err(MailboxDomainError::InvalidDispatchSequence)
        );
        let sequence = DispatchSequence::new(1).expect("sequence");
        assert_eq!(
            DeliveryAttempt::new(
                DeliveryAttemptId::new(),
                MailboxEventId::new(),
                0,
                sequence,
                StateAccessOutcome::NoState
            ),
            Err(MailboxDomainError::InvalidAttemptNumber)
        );
    }

    #[test]
    fn operation_ids_are_deterministic_and_operation_specific() {
        let mailbox = MailboxId::from_uuid(Uuid::from_u128(1));
        let event = MailboxEventId::from_uuid(Uuid::from_u128(2));
        let first = MailboxOperationIdentity::retry(mailbox, event, 2).id();
        assert_eq!(
            first,
            MailboxOperationIdentity::retry(mailbox, event, 2).id()
        );
        assert_ne!(
            first,
            MailboxOperationIdentity::retry(mailbox, event, 3).id()
        );
        assert_ne!(
            first,
            MailboxOperationIdentity::dead_letter(mailbox, event, 2).id()
        );
        assert_ne!(
            MailboxOperationIdentity::dispatch(mailbox, event, 1).id(),
            MailboxOperationIdentity::dispatch(mailbox, event, 2).id(),
            "each retry needs a fresh durable dispatch command"
        );
        let publish = MailboxOperationIdentity::publish(
            mailbox,
            ProducerId::parse("gateway:telegram").expect("producer"),
            DeduplicationKey::parse("update-123").expect("key"),
        );
        assert_ne!(first, publish.id());
    }

    #[test]
    fn malformed_deserialization_is_rejected() {
        let parsed: Result<EnvelopeMethod, _> = serde_json::from_str("\"get\"");
        assert!(parsed.is_err());
        let parsed: Result<DispatchSequence, _> = serde_json::from_str("0");
        assert!(parsed.is_err());
    }
}
