use thiserror::Error;

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
