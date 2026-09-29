//! Stable encrypted-envelope metadata types.

use secret_domain::{SecretId, SecretOwner, SecretVersionId};

/// Algorithm identifier persisted with every encrypted version.
pub const ALGORITHM: &str = "AES-256-GCM+AES-256-GCM-KW/v1";

/// Immutable metadata authenticated alongside a secret version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionContext {
    /// Exact organization/project owner.
    pub owner: SecretOwner,
    /// Parent secret.
    pub secret_id: SecretId,
    /// Immutable version.
    pub version_id: SecretVersionId,
    /// Monotonic version sequence.
    pub sequence: u64,
    /// Non-sensitive content type.
    pub media_type: String,
}

impl VersionContext {
    pub(crate) fn associated_data(&self) -> Vec<u8> {
        let mut data = Vec::with_capacity(160);
        append_field(&mut data, ALGORITHM.as_bytes());
        match self.owner {
            SecretOwner::Organization(id) => {
                append_field(&mut data, b"organization");
                append_field(&mut data, id.as_uuid().as_bytes());
            }
            SecretOwner::Project(id) => {
                append_field(&mut data, b"project");
                append_field(&mut data, id.as_uuid().as_bytes());
            }
        }
        append_field(&mut data, self.secret_id.as_uuid().as_bytes());
        append_field(&mut data, self.version_id.as_uuid().as_bytes());
        append_field(&mut data, &self.sequence.to_be_bytes());
        append_field(&mut data, self.media_type.as_bytes());
        data
    }
}

pub fn append_field(output: &mut Vec<u8>, value: &[u8]) {
    output.extend_from_slice(&value.len().to_be_bytes());
    output.extend_from_slice(value);
}

/// Complete encrypted envelope safe for durable storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncryptedSecretVersion {
    /// Immutable secret version.
    pub version_id: SecretVersionId,
    /// Versioned algorithm.
    pub algorithm: String,
    /// Host key identifier used to wrap this version's random data key.
    pub key_reference: String,
    /// Unique content-encryption nonce.
    pub data_nonce: [u8; 12],
    /// Authenticated ciphertext with tag.
    pub ciphertext: Vec<u8>,
    /// Unique wrapping nonce.
    pub wrap_nonce: [u8; 12],
    /// Authenticated wrapped per-version data key with tag.
    pub wrapped_data_key: Vec<u8>,
    /// Hash of authenticated associated data for inspection and backup checks.
    pub associated_data_hash: [u8; 32],
    /// Plaintext length.
    pub content_length: u32,
}
