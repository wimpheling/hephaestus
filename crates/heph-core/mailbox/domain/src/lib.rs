//! Provider-neutral durable mailbox envelopes, delivery state, and identities.
//!
//! Mailbox payloads remain opaque to the control plane. This crate validates
//! their bounded metadata and defines the logical state machine; `PostgreSQL`,
//! NATS, and VM adapters implement durable storage and execution separately.

use runtime_types::AgentInstanceId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fmt, str::FromStr};
use thiserror::Error;
use time::OffsetDateTime;
use uuid::Uuid;

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

macro_rules! identifier {
    ($name:ident, $documentation:literal) => {
        #[doc = $documentation]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            /// Creates a random version 4 identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }

            /// Creates an identifier from its UUID representation.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }

            /// Returns the UUID representation.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(value).map(Self)
            }
        }
    };
}

identifier!(
    MailboxId,
    "A stable identifier for one agent-instance-owned mailbox."
);
identifier!(
    MailboxEventId,
    "A stable identifier for one accepted mailbox event."
);
identifier!(
    DeliveryAttemptId,
    "A stable identifier for one logical delivery attempt."
);
identifier!(
    BodyReferenceId,
    "An opaque identifier for one accepted mailbox body."
);
identifier!(
    MailboxOperationId,
    "A deterministic idempotency identity for one mailbox operation."
);

/// A stable, normalized producer identity within a mailbox's declared scope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProducerId(String);

impl ProducerId {
    /// Parses a bounded, printable opaque producer key.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidProducer`] for empty, oversized,
    /// control-character, or surrounding-whitespace values.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_PRODUCER_KEY_BYTES,
            MailboxDomainError::InvalidProducer,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated opaque key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProducerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for ProducerId {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<ProducerId> for String {
    fn from(value: ProducerId) -> Self {
        value.0
    }
}

/// An opaque stable deduplication key selected by a producer.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct DeduplicationKey(String);

impl DeduplicationKey {
    /// Parses a bounded opaque idempotency key.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidDeduplicationKey`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_DEDUPLICATION_KEY_BYTES,
            MailboxDomainError::InvalidDeduplicationKey,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated opaque key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeduplicationKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for DeduplicationKey {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<DeduplicationKey> for String {
    fn from(value: DeduplicationKey) -> Self {
        value.0
    }
}

/// A normalized upper-case HTTP-like method carried by an opaque envelope.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EnvelopeMethod(String);

impl EnvelopeMethod {
    /// Parses an upper-case token method.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidMethod`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_METHOD_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(MailboxDomainError::InvalidMethod);
        }
        Ok(Self(value))
    }

    /// Returns the normalized method.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvelopeMethod {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for EnvelopeMethod {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<EnvelopeMethod> for String {
    fn from(value: EnvelopeMethod) -> Self {
        value.0
    }
}

/// A bounded absolute route without a URI scheme, authority, or fragment.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct EnvelopeRoute(String);

impl EnvelopeRoute {
    /// Parses one bounded relative route.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidRoute`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if !(1..=MAX_ROUTE_BYTES).contains(&value.len())
            || !value.starts_with('/')
            || value.starts_with("//")
            || value.contains('#')
            || value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
        {
            return Err(MailboxDomainError::InvalidRoute);
        }
        Ok(Self(value))
    }

    /// Returns the validated route.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for EnvelopeRoute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for EnvelopeRoute {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<EnvelopeRoute> for String {
    fn from(value: EnvelopeRoute) -> Self {
        value.0
    }
}

/// A lower-case selected HTTP header name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SelectedHeaderName(String);

impl SelectedHeaderName {
    /// Parses a lower-case ASCII token name.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidHeaderName`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > MAX_HEADER_NAME_BYTES
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(MailboxDomainError::InvalidHeaderName);
        }
        Ok(Self(value))
    }

    /// Returns the normalized header name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SelectedHeaderName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}
impl TryFrom<String> for SelectedHeaderName {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<SelectedHeaderName> for String {
    fn from(value: SelectedHeaderName) -> Self {
        value.0
    }
}

/// One bounded selected header value.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct SelectedHeaderValue(String);

impl SelectedHeaderValue {
    /// Parses a bounded header value without control characters.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidHeaderValue`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        if value.len() > MAX_HEADER_VALUE_BYTES || value.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(MailboxDomainError::InvalidHeaderValue);
        }
        Ok(Self(value))
    }

    /// Returns the validated header value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for SelectedHeaderValue {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<SelectedHeaderValue> for String {
    fn from(value: SelectedHeaderValue) -> Self {
        value.0
    }
}

/// Exact integrity metadata for an opaque accepted body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyReference {
    /// Opaque body identity; commands carry this rather than payload bytes.
    pub id: BodyReferenceId,
    /// Exact stored body length.
    pub byte_length: u32,
    /// SHA-256 integrity hash of the stored bytes.
    pub integrity_hash: [u8; BODY_INTEGRITY_HASH_BYTES],
}

impl BodyReference {
    /// Creates validated body metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::BodyTooLarge`] when the supplied size
    /// exceeds the platform's bounded mailbox body limit.
    pub const fn new(
        id: BodyReferenceId,
        byte_length: u32,
        integrity_hash: [u8; BODY_INTEGRITY_HASH_BYTES],
    ) -> Result<Self, MailboxDomainError> {
        if byte_length > MAX_BODY_BYTES {
            return Err(MailboxDomainError::BodyTooLarge);
        }
        Ok(Self {
            id,
            byte_length,
            integrity_hash,
        })
    }
}

/// Bounded, opaque content metadata attached to a mailbox body reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentMetadata {
    /// Referenced payload, never inline payload bytes.
    pub body: BodyReference,
    /// Declared content type, if one was supplied.
    pub content_type: Option<String>,
    /// Declared content encoding, if one was supplied.
    pub content_encoding: Option<String>,
}

impl ContentMetadata {
    /// Creates bounded content metadata.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidContentMetadata`] for blank,
    /// oversized, or control-character metadata.
    pub fn new(
        body: BodyReference,
        content_type: Option<String>,
        content_encoding: Option<String>,
    ) -> Result<Self, MailboxDomainError> {
        for value in [content_type.as_deref(), content_encoding.as_deref()]
            .into_iter()
            .flatten()
        {
            validate_opaque(
                value,
                MAX_CONTENT_METADATA_BYTES,
                MailboxDomainError::InvalidContentMetadata,
            )?;
        }
        Ok(Self {
            body,
            content_type,
            content_encoding,
        })
    }
}

/// A bounded opaque distributed trace context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct TraceContext(String);

impl TraceContext {
    /// Parses a bounded opaque trace context.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::InvalidTraceContext`] for malformed input.
    pub fn parse(value: impl Into<String>) -> Result<Self, MailboxDomainError> {
        let value = value.into();
        validate_opaque(
            &value,
            MAX_TRACE_CONTEXT_BYTES,
            MailboxDomainError::InvalidTraceContext,
        )?;
        Ok(Self(value))
    }

    /// Returns the validated trace context.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for TraceContext {
    type Error = MailboxDomainError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}
impl From<TraceContext> for String {
    fn from(value: TraceContext) -> Self {
        value.0
    }
}

/// Bounded provider-neutral metadata accepted for one mailbox event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailboxEnvelope {
    /// Opaque transport method.
    pub method: EnvelopeMethod,
    /// Opaque bounded route.
    pub route: EnvelopeRoute,
    /// Allowlisted selected headers, sorted by canonical lower-case name.
    pub headers: BTreeMap<SelectedHeaderName, SelectedHeaderValue>,
    /// Payload metadata and opaque reference.
    pub content: ContentMetadata,
    /// Platform receive timestamp.
    pub received_at: OffsetDateTime,
    /// Optional opaque trace context.
    pub trace_context: Option<TraceContext>,
}

impl MailboxEnvelope {
    /// Creates a bounded envelope.
    ///
    /// # Errors
    ///
    /// Returns [`MailboxDomainError::TooManyHeaders`] when more than the
    /// bounded selected-header allowance is supplied.
    pub fn new(
        method: EnvelopeMethod,
        route: EnvelopeRoute,
        headers: BTreeMap<SelectedHeaderName, SelectedHeaderValue>,
        content: ContentMetadata,
        received_at: OffsetDateTime,
        trace_context: Option<TraceContext>,
    ) -> Result<Self, MailboxDomainError> {
        if headers.len() > MAX_HEADERS {
            return Err(MailboxDomainError::TooManyHeaders);
        }
        Ok(Self {
            method,
            route,
            headers,
            content,
            received_at,
            trace_context,
        })
    }
}

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

/// Validation failures for provider-neutral mailbox values.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MailboxDomainError {
    /// A producer identity was malformed.
    #[error("mailbox producer identity is invalid")]
    InvalidProducer,
    /// A deduplication key was malformed.
    #[error("mailbox deduplication key is invalid")]
    InvalidDeduplicationKey,
    /// A method was malformed.
    #[error("mailbox method is invalid")]
    InvalidMethod,
    /// A route was malformed.
    #[error("mailbox route is invalid")]
    InvalidRoute,
    /// A selected header name was malformed.
    #[error("mailbox header name is invalid")]
    InvalidHeaderName,
    /// A selected header value was malformed.
    #[error("mailbox header value is invalid")]
    InvalidHeaderValue,
    /// More than the bounded header limit was supplied.
    #[error("mailbox envelope has too many selected headers")]
    TooManyHeaders,
    /// Content metadata was malformed.
    #[error("mailbox content metadata is invalid")]
    InvalidContentMetadata,
    /// Trace context was malformed.
    #[error("mailbox trace context is invalid")]
    InvalidTraceContext,
    /// A body exceeds the accepted mailbox body bound.
    #[error("mailbox body exceeds the maximum accepted size")]
    BodyTooLarge,
    /// A dispatch sequence must be non-zero.
    #[error("mailbox dispatch sequence must be non-zero")]
    InvalidDispatchSequence,
    /// A delivery attempt number must be one-based.
    #[error("mailbox attempt number must be one-based")]
    InvalidAttemptNumber,
}

fn validate_opaque(
    value: &str,
    maximum: usize,
    error: MailboxDomainError,
) -> Result<(), MailboxDomainError> {
    if value.is_empty()
        || value.len() > maximum
        || value.trim() != value
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(error);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
