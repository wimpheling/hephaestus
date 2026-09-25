use mailbox_dispatch::MailboxDispatchStoreError;

/// Non-disclosing mailbox persistence failure.
#[derive(Debug, thiserror::Error)]
pub enum MailboxPersistenceError {
    /// The mailbox is unavailable or a duplicate cannot be read back safely.
    #[error("mailbox is unavailable")]
    Unavailable,
    /// The presented encoded body does not match its immutable body reference.
    #[error("mailbox payload integrity validation failed")]
    PayloadIntegrity,
    /// The bounded MVP-02 payload store deliberately accepts identity only.
    #[error("mailbox payload content encoding is unsupported")]
    UnsupportedContentEncoding,
    /// `PostgreSQL` or serialization failed without exposing request content.
    #[error("mailbox persistence failed: {0}")]
    Provider(String),
    /// The same retry identity was used for a different mailbox allocation.
    #[error("mailbox allocation idempotency identity conflicts")]
    IdempotencyConflict,
}

pub fn storage(error: impl std::fmt::Display) -> MailboxPersistenceError {
    MailboxPersistenceError::Provider(error.to_string())
}

pub fn dispatch_error(error: impl std::fmt::Display) -> MailboxDispatchStoreError {
    MailboxDispatchStoreError(error.to_string())
}
