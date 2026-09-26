//! Provider-neutral errors for secret storage.

/// Provider-neutral encrypted storage failure. Errors never include plaintext,
/// ciphertext, nonce, or key material.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SecretStoreError {
    /// No valid active key is provisioned.
    #[error("no active secret encryption key is available")]
    NoActiveKey,
    /// An exact versioned key cannot be loaded.
    #[error("required secret encryption key is unavailable")]
    UnavailableKey,
    /// Key reference format is invalid.
    #[error("secret encryption key reference is invalid")]
    InvalidKeyReference,
    /// A key does not contain exactly 256 bits.
    #[error("secret encryption key must contain exactly 32 bytes")]
    InvalidKeyLength,
    /// Key references must be unique.
    #[error("secret encryption key reference is duplicated")]
    DuplicateKeyReference,
    /// The active key cannot be removed before another is activated.
    #[error("active secret encryption key cannot be removed")]
    ActiveKeyRemoval,
    /// Immutable authenticated context does not match the record.
    #[error("encrypted secret context does not match the requested version")]
    ContextMismatch,
    /// Authenticated decryption rejected tampered or wrong-key material.
    #[error("encrypted secret authentication failed")]
    Authentication,
    /// Non-sensitive metadata is malformed.
    #[error("encrypted secret metadata is invalid")]
    InvalidMetadata,
    /// Cryptographic primitive initialization or encryption failed.
    #[error("secret encryption operation failed")]
    Crypto,
}
