use sha2::{Digest, Sha256};
use std::fmt;

/// SHA-256 digest of a normalized capability scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GitCapabilityHash([u8; 32]);

impl GitCapabilityHash {
    /// Restores a digest read from trusted immutable persistence.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the raw digest.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for GitCapabilityHash {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

pub fn git_hash(bytes: &[u8]) -> GitCapabilityHash {
    let digest = Sha256::digest(bytes);
    let mut hash = [0_u8; 32];
    hash.copy_from_slice(&digest);
    GitCapabilityHash(hash)
}
