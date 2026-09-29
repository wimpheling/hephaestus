use serde::{Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;

use super::{MAX_SECRET_VALUE_BYTES, SecretValueError};

/// A plaintext value that is impossible to expose through formatting or
/// serialization.
///
/// Callers should keep this value short-lived and pass it directly to an
/// encrypted store or ephemeral runtime materializer.
pub struct SecretValue(Vec<u8>);

impl SecretValue {
    /// Accepts bounded non-empty plaintext.
    ///
    /// # Errors
    ///
    /// Returns [`SecretValueError::InvalidSecretSize`] for empty or oversized
    /// input.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, SecretValueError> {
        let value = value.into();
        if value.is_empty() || value.len() > MAX_SECRET_VALUE_BYTES {
            return Err(SecretValueError::InvalidSecretSize);
        }
        Ok(Self(value))
    }

    /// Exposes the plaintext only to the narrow storage or delivery boundary.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    /// Returns the non-sensitive plaintext length.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns whether the value is empty.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl Drop for SecretValue {
    fn drop(&mut self) {
        // A volatile write makes best effort to keep the wipe from being
        // optimized away. Stronger locked-memory handling belongs at the
        // concrete key-provider boundary.
        for byte in &mut self.0 {
            // SAFETY is intentionally avoided because this workspace denies
            // unsafe code; black_box retains the observable writes.
            *std::hint::black_box(byte) = 0;
        }
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

impl fmt::Display for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Serialize for SecretValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

/// A bounded opaque runtime credential. Its stored representation is a hash,
/// never this bearer value.
pub struct OpaqueRuntimeCredential(Vec<u8>);

impl OpaqueRuntimeCredential {
    /// Accepts a credential with at least 32 bytes of entropy and at most 256
    /// bytes.
    ///
    /// # Errors
    ///
    /// Returns [`SecretValueError::InvalidCredentialSize`] when out of bounds.
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, SecretValueError> {
        let value = value.into();
        if !(32..=256).contains(&value.len()) {
            return Err(SecretValueError::InvalidCredentialSize);
        }
        Ok(Self(value))
    }

    /// Returns a one-way SHA-256 representation suitable for durable storage.
    #[must_use]
    pub fn storage_hash(&self) -> [u8; 32] {
        Sha256::digest(&self.0).into()
    }

    /// Exposes the bearer credential at the transport authentication boundary.
    #[must_use]
    pub fn expose(&self) -> &[u8] {
        &self.0
    }
}

impl fmt::Debug for OpaqueRuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OpaqueRuntimeCredential([REDACTED])")
    }
}

impl fmt::Display for OpaqueRuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}
