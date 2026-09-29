use serde::{Deserialize, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use time::OffsetDateTime;

use crate::{
    AUTHORITY_HASH_BYTES, AuthorizationSnapshot, CapabilityError, CapabilityOperation,
    CapabilityResource, RUNTIME_CREDENTIAL_BYTES, RuntimeSessionId, RuntimeSessionIdentity,
};

const RUNTIME_CREDENTIAL_HASH_DOMAIN: &[u8] = b"hephaestus.runtime-credential-verifier.v1\0";
const REDACTED: &str = "[REDACTED]";

/// Stable positive generation for one runtime credential handoff.
///
/// A session starts at generation one. Redelivery uses that same generation;
/// rotating bearer material requires a different runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeCredentialGeneration(u64);

impl RuntimeCredentialGeneration {
    /// The only generation currently issued for a new runtime session.
    pub const INITIAL: Self = Self(1);

    /// Constructs a positive generation.
    ///
    /// # Errors
    ///
    /// Returns [`CapabilityError::InvalidCredentialGeneration`] for zero.
    pub const fn new(value: u64) -> Result<Self, CapabilityError> {
        if value == 0 {
            Err(CapabilityError::InvalidCredentialGeneration)
        } else {
            Ok(Self(value))
        }
    }

    /// Returns the stored integer representation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Opaque runtime bearer material.
///
/// This value is deliberately neither cloneable nor deserializable. Durable
/// stores receive only [`RuntimeCredentialHash`].
pub struct RuntimeCredential([u8; RUNTIME_CREDENTIAL_BYTES]);

impl RuntimeCredential {
    /// Wraps 256 bits produced by a cryptographically secure random source.
    #[must_use]
    pub const fn from_secret(secret: [u8; RUNTIME_CREDENTIAL_BYTES]) -> Self {
        Self(secret)
    }

    /// Exposes bearer material only at the authenticated bootstrap boundary.
    #[must_use]
    pub const fn expose(&self) -> &[u8; RUNTIME_CREDENTIAL_BYTES] {
        &self.0
    }

    /// Derives the only representation permitted in durable storage.
    #[must_use]
    pub fn storage_hash(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> RuntimeCredentialHash {
        let mut digest = Sha256::new();
        digest.update(RUNTIME_CREDENTIAL_HASH_DOMAIN);
        digest.update(session_id.as_uuid().as_bytes());
        digest.update(generation.get().to_be_bytes());
        digest.update(self.0);
        RuntimeCredentialHash(digest.finalize().into())
    }
}

impl Drop for RuntimeCredential {
    fn drop(&mut self) {
        for byte in &mut self.0 {
            *std::hint::black_box(byte) = 0;
        }
    }
}

impl fmt::Debug for RuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeCredential([REDACTED])")
    }
}

impl fmt::Display for RuntimeCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTED)
    }
}

impl Serialize for RuntimeCredential {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(REDACTED)
    }
}

/// Domain-separated one-way verifier for a runtime credential.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RuntimeCredentialHash([u8; 32]);

impl RuntimeCredentialHash {
    /// Reconstructs a verifier loaded from trusted durable storage.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the bytes safe for durable storage.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }

    /// Checks candidate bearer material without an early exit on digest bytes.
    #[must_use]
    pub fn verifies(
        self,
        credential: &RuntimeCredential,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> bool {
        let candidate = credential.storage_hash(session_id, generation);
        self.0
            .iter()
            .zip(candidate.0.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

impl fmt::Debug for RuntimeCredentialHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeCredentialHash([REDACTED])")
    }
}

/// Lifecycle of one short-lived generic runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSessionStatus {
    /// Credential exists in a host-only handoff envelope but is not acknowledged.
    PendingHandoff,
    /// The guest acknowledged the exact issuance generation.
    Active,
    /// Authority was permanently revoked.
    Revoked,
    /// The session reached its expiry.
    Expired,
}

impl RuntimeSessionStatus {
    /// Returns whether a lifecycle transition is valid.
    #[must_use]
    pub const fn can_transition_to(self, next: Self) -> bool {
        matches!(
            (self, next),
            (
                Self::PendingHandoff,
                Self::Active | Self::Revoked | Self::Expired
            ) | (Self::Active, Self::Revoked | Self::Expired)
        )
    }
}

/// A verified runtime identity paired with its immutable authority ceiling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeAuthority {
    identity: RuntimeSessionIdentity,
    snapshot: AuthorizationSnapshot,
}

impl RuntimeAuthority {
    /// Pairs identity claims with the exact referenced snapshot.
    ///
    /// # Errors
    ///
    /// Rejects a different snapshot identifier, hash, or workload revision.
    pub fn new(
        identity: RuntimeSessionIdentity,
        snapshot: AuthorizationSnapshot,
    ) -> Result<Self, CapabilityError> {
        if identity.snapshot_id != snapshot.id
            || identity.snapshot_hash != snapshot.normalized_hash
            || identity.principal != snapshot.principal
        {
            return Err(CapabilityError::SnapshotIdentityMismatch);
        }
        Ok(Self { identity, snapshot })
    }

    /// Returns the exact authenticated runtime identity.
    #[must_use]
    pub const fn identity(&self) -> &RuntimeSessionIdentity {
        &self.identity
    }

    /// Returns the immutable authorization snapshot.
    #[must_use]
    pub const fn snapshot(&self) -> &AuthorizationSnapshot {
        &self.snapshot
    }

    /// Returns whether the request is inside the immutable ceiling and the
    /// runtime identity is temporally valid.
    ///
    /// A true result is not a live authorization decision. The caller must
    /// still check current authorization and resource lifecycle state.
    #[must_use]
    pub fn permits_at(
        &self,
        now: OffsetDateTime,
        resource: CapabilityResource,
        operation: CapabilityOperation,
    ) -> bool {
        self.identity.is_valid_at(now) && self.snapshot.allows(resource, operation)
    }
}

/// Stable SHA-256 digest of one normalized authority value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AuthorityHash(pub(super) [u8; AUTHORITY_HASH_BYTES]);

impl AuthorityHash {
    /// Reconstructs a hash loaded from trusted durable storage.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; AUTHORITY_HASH_BYTES]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; AUTHORITY_HASH_BYTES] {
        &self.0
    }
}

impl fmt::Display for AuthorityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}
