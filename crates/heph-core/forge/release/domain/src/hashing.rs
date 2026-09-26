use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// SHA-256 content or normalized-document hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash([u8; 32]);

impl ContentHash {
    /// Hashes exact bytes.
    #[must_use]
    pub fn digest(value: &[u8]) -> Self {
        Self(Sha256::digest(value).into())
    }

    /// Wraps an already computed SHA-256 digest.
    #[must_use]
    pub const fn from_digest(value: [u8; 32]) -> Self {
        Self(value)
    }

    /// Returns raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Deterministic identity for build, publication, revision, attachment,
/// update, and run commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReleaseCommandKey([u8; 32]);

impl ReleaseCommandKey {
    /// Derives an unambiguous operation-scoped identity.
    #[must_use]
    pub fn derive(operation: &str, fields: &[&[u8]]) -> Self {
        let mut digest = Sha256::new();
        update_field(&mut digest, operation.as_bytes());
        for field in fields {
            update_field(&mut digest, field);
        }
        Self(digest.finalize().into())
    }

    /// Returns raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

fn update_field(digest: &mut Sha256, value: &[u8]) {
    digest.update(value.len().to_be_bytes());
    digest.update(value);
}
