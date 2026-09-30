use super::{RUNTIME_GIT_CREDENTIAL_BYTES, RuntimeGitAuthorityError};
use sha2::{Digest, Sha256};
use std::fmt;
use zeroize::{Zeroize, Zeroizing};

const CREDENTIAL_PREFIX: &str = "heph_git_v1_";
const HASH_DOMAIN: &[u8] = b"hephaestus.runtime-git-credential-verifier.v1\0";

/// One separately discriminated runtime Git bearer.
pub struct RuntimeGitCredential([u8; RUNTIME_GIT_CREDENTIAL_BYTES]);

impl RuntimeGitCredential {
    /// Creates a credential from cryptographically random secret bytes.
    #[must_use]
    pub const fn from_secret(secret: [u8; RUNTIME_GIT_CREDENTIAL_BYTES]) -> Self {
        Self(secret)
    }

    /// Parses the canonical password representation accepted by Git HTTP.
    ///
    /// # Errors
    ///
    /// Returns a redacted error for malformed or incorrectly sized input.
    pub fn parse(value: &str) -> Result<Self, RuntimeGitAuthorityError> {
        use base64::Engine as _;

        let encoded = value
            .strip_prefix(CREDENTIAL_PREFIX)
            .ok_or(RuntimeGitAuthorityError::InvalidCredential)?;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| RuntimeGitAuthorityError::InvalidCredential)?;
        let secret = decoded
            .try_into()
            .map_err(|_| RuntimeGitAuthorityError::InvalidCredential)?;
        Ok(Self(secret))
    }

    /// Returns the canonical password representation in zeroizing storage.
    #[must_use]
    pub fn expose_token(&self) -> Zeroizing<String> {
        use base64::Engine as _;

        Zeroizing::new(format!(
            "{CREDENTIAL_PREFIX}{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.0)
        ))
    }

    /// Exposes raw bearer bytes only to authenticated bootstrap conversion.
    #[must_use]
    pub const fn expose(&self) -> &[u8; RUNTIME_GIT_CREDENTIAL_BYTES] {
        &self.0
    }

    /// Computes the domain-separated durable verifier.
    #[must_use]
    pub fn storage_hash(&self) -> RuntimeGitCredentialHash {
        let mut digest = Sha256::new();
        digest.update(HASH_DOMAIN);
        digest.update(self.0);
        RuntimeGitCredentialHash(digest.finalize().into())
    }
}

impl fmt::Debug for RuntimeGitCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeGitCredential([REDACTED])")
    }
}

impl Drop for RuntimeGitCredential {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// Hash-only verifier persisted for one runtime Git bearer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeGitCredentialHash([u8; 32]);

impl RuntimeGitCredentialHash {
    /// Returns verifier bytes for persistence or indexed lookup.
    #[must_use]
    pub const fn as_bytes(self) -> [u8; 32] {
        self.0
    }
}
