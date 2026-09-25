//! Authenticated envelope encryption and the provider-neutral secret storage
//! boundary.
//!
//! `PostgreSQL` stores [`EncryptedSecretVersion`] metadata and ciphertext. The
//! versioned host key provider remains outside `PostgreSQL`. A unique random data
//! key and unique nonces isolate every immutable secret version.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use secret_domain::{SecretId, SecretOwner, SecretValue, SecretVersionId};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use zeroize::Zeroizing;

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
    fn associated_data(&self) -> Vec<u8> {
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

fn append_field(output: &mut Vec<u8>, value: &[u8]) {
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

/// Versioned host-side key provider used only for data-key wrapping.
pub trait KeyProvider {
    /// Returns the current key reference for new versions.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::NoActiveKey`] when startup provisioning is
    /// incomplete.
    fn active_key_reference(&self) -> Result<&str, SecretStoreError>;

    /// Returns exact 256-bit key material for a versioned reference.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::UnavailableKey`] instead of falling back.
    fn key(&self, reference: &str) -> Result<Zeroizing<[u8; 32]>, SecretStoreError>;
}

/// In-process provider suitable for host-loaded key material, tests, and
/// development. Production KMS implementations can implement [`KeyProvider`]
/// without changing encrypted records.
#[derive(Clone)]
pub struct LocalKeyProvider {
    active_reference: String,
    keys: BTreeMap<String, Zeroizing<[u8; 32]>>,
}

impl LocalKeyProvider {
    /// Builds and validates a versioned key set.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError`] when references are malformed, keys are not
    /// exact 32-byte values, or the active reference is unavailable.
    pub fn new<I, K, V>(
        active_reference: impl Into<String>,
        keys: I,
    ) -> Result<Self, SecretStoreError>
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<String>,
        V: AsRef<[u8]>,
    {
        let active_reference = active_reference.into();
        let mut parsed = BTreeMap::new();
        for (reference, bytes) in keys {
            let reference = reference.into();
            if !valid_key_reference(&reference) {
                return Err(SecretStoreError::InvalidKeyReference);
            }
            let bytes: [u8; 32] = bytes
                .as_ref()
                .try_into()
                .map_err(|_| SecretStoreError::InvalidKeyLength)?;
            if parsed.insert(reference, Zeroizing::new(bytes)).is_some() {
                return Err(SecretStoreError::DuplicateKeyReference);
            }
        }
        if !parsed.contains_key(&active_reference) {
            return Err(SecretStoreError::NoActiveKey);
        }
        Ok(Self {
            active_reference,
            keys: parsed,
        })
    }

    /// Changes the wrapping key used for later encryption while retaining old
    /// keys for restore/decryption.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::UnavailableKey`] for an unknown reference.
    pub fn rotate_active_key(&mut self, reference: &str) -> Result<(), SecretStoreError> {
        if !self.keys.contains_key(reference) {
            return Err(SecretStoreError::UnavailableKey);
        }
        reference.clone_into(&mut self.active_reference);
        Ok(())
    }

    /// Removes a retired key. Existing versions under that key subsequently
    /// fail closed until a backup restores it.
    ///
    /// # Errors
    ///
    /// Returns [`SecretStoreError::ActiveKeyRemoval`] for the active key.
    pub fn remove_key(&mut self, reference: &str) -> Result<(), SecretStoreError> {
        if reference == self.active_reference {
            return Err(SecretStoreError::ActiveKeyRemoval);
        }
        self.keys.remove(reference);
        Ok(())
    }
}

impl KeyProvider for LocalKeyProvider {
    fn active_key_reference(&self) -> Result<&str, SecretStoreError> {
        if self.keys.contains_key(&self.active_reference) {
            Ok(&self.active_reference)
        } else {
            Err(SecretStoreError::NoActiveKey)
        }
    }

    fn key(&self, reference: &str) -> Result<Zeroizing<[u8; 32]>, SecretStoreError> {
        self.keys
            .get(reference)
            .map(|value| Zeroizing::new(**value))
            .ok_or(SecretStoreError::UnavailableKey)
    }
}

fn valid_key_reference(value: &str) -> bool {
    (1..=128).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'/'))
}

/// Narrow encrypted secret store. Plaintext enters only through [`Self::seal`]
/// and exits only through [`Self::resolve`].
#[derive(Clone)]
pub struct EncryptedStore<K> {
    keys: K,
}

impl<K: KeyProvider> EncryptedStore<K> {
    /// Creates a store after concrete host key provisioning.
    #[must_use]
    pub const fn new(keys: K) -> Self {
        Self { keys }
    }

    /// Returns the key provider for rotation/operational inspection.
    #[must_use]
    pub const fn key_provider(&self) -> &K {
        &self.keys
    }

    /// Authenticated-encrypts one immutable version using a random isolated
    /// data key.
    ///
    /// # Errors
    ///
    /// Fails closed for missing keys, invalid metadata bounds, or
    /// cryptographic failure.
    pub fn seal(
        &self,
        context: &VersionContext,
        value: &SecretValue,
    ) -> Result<EncryptedSecretVersion, SecretStoreError> {
        if context.media_type.is_empty() || context.media_type.len() > 128 {
            return Err(SecretStoreError::InvalidMetadata);
        }
        let key_reference = self.keys.active_key_reference()?.to_owned();
        let wrapping_key = self.keys.key(&key_reference)?;
        let data_key: Zeroizing<[u8; 32]> =
            Zeroizing::new(Aes256Gcm::generate_key(&mut OsRng).into());
        let data_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let wrap_nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let associated_data = context.associated_data();

        let data_cipher =
            Aes256Gcm::new_from_slice(data_key.as_ref()).map_err(|_| SecretStoreError::Crypto)?;
        let ciphertext = data_cipher
            .encrypt(
                &data_nonce,
                Payload {
                    msg: value.expose(),
                    aad: &associated_data,
                },
            )
            .map_err(|_| SecretStoreError::Crypto)?;

        let wrapping_cipher = Aes256Gcm::new_from_slice(wrapping_key.as_ref())
            .map_err(|_| SecretStoreError::Crypto)?;
        let wrapping_aad = wrapping_associated_data(context.version_id, &key_reference);
        let wrapped_data_key = wrapping_cipher
            .encrypt(
                &wrap_nonce,
                Payload {
                    msg: data_key.as_ref(),
                    aad: &wrapping_aad,
                },
            )
            .map_err(|_| SecretStoreError::Crypto)?;

        Ok(EncryptedSecretVersion {
            version_id: context.version_id,
            algorithm: String::from(ALGORITHM),
            key_reference,
            data_nonce: data_nonce.into(),
            ciphertext,
            wrap_nonce: wrap_nonce.into(),
            wrapped_data_key,
            associated_data_hash: Sha256::digest(&associated_data).into(),
            content_length: u32::try_from(value.len())
                .map_err(|_| SecretStoreError::InvalidMetadata)?,
        })
    }

    /// Resolves an already-authorized exact version into a short-lived
    /// redacted plaintext wrapper.
    ///
    /// Authorization and lifecycle checks intentionally happen before this
    /// narrow resolver is called.
    ///
    /// # Errors
    ///
    /// Fails closed on unavailable keys, mismatched context, tampering, or
    /// unsupported algorithms.
    pub fn resolve(
        &self,
        context: &VersionContext,
        encrypted: &EncryptedSecretVersion,
    ) -> Result<SecretValue, SecretStoreError> {
        if encrypted.algorithm != ALGORITHM || encrypted.version_id != context.version_id {
            return Err(SecretStoreError::ContextMismatch);
        }
        let associated_data = context.associated_data();
        if encrypted.associated_data_hash != Sha256::digest(&associated_data).as_slice() {
            return Err(SecretStoreError::ContextMismatch);
        }
        let wrapping_key = self.keys.key(&encrypted.key_reference)?;
        let wrapping_cipher = Aes256Gcm::new_from_slice(wrapping_key.as_ref())
            .map_err(|_| SecretStoreError::Crypto)?;
        let wrapping_aad = wrapping_associated_data(context.version_id, &encrypted.key_reference);
        let wrapped_nonce = encrypted.wrap_nonce.into();
        let data_key = Zeroizing::new(
            wrapping_cipher
                .decrypt(
                    &wrapped_nonce,
                    Payload {
                        msg: &encrypted.wrapped_data_key,
                        aad: &wrapping_aad,
                    },
                )
                .map_err(|_| SecretStoreError::Authentication)?,
        );
        if data_key.len() != 32 {
            return Err(SecretStoreError::Authentication);
        }
        let data_cipher =
            Aes256Gcm::new_from_slice(&data_key).map_err(|_| SecretStoreError::Crypto)?;
        let data_nonce = encrypted.data_nonce.into();
        let plaintext = data_cipher
            .decrypt(
                &data_nonce,
                Payload {
                    msg: &encrypted.ciphertext,
                    aad: &associated_data,
                },
            )
            .map_err(|_| SecretStoreError::Authentication)?;
        if plaintext.len() != encrypted.content_length as usize {
            return Err(SecretStoreError::Authentication);
        }
        SecretValue::new(plaintext).map_err(|_| SecretStoreError::InvalidMetadata)
    }
}

fn wrapping_associated_data(version_id: SecretVersionId, key_reference: &str) -> Vec<u8> {
    let mut value = Vec::with_capacity(128);
    append_field(&mut value, b"hephaestus-secret-data-key/v1");
    append_field(&mut value, version_id.as_uuid().as_bytes());
    append_field(&mut value, key_reference.as_bytes());
    value
}

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

#[cfg(test)]
mod tests {
    use super::{
        EncryptedSecretVersion, EncryptedStore, LocalKeyProvider, SecretStoreError, VersionContext,
    };
    use forge_domain::ProjectId;
    use secret_domain::{SecretId, SecretOwner, SecretValue, SecretVersionId};

    const SENTINEL: &[u8] = b"top-secret-sentinel-cf58e2a4";

    fn provider(active: &str) -> LocalKeyProvider {
        LocalKeyProvider::new(
            active,
            [
                ("key/v1", [1_u8; 32]),
                ("key/v2", [2_u8; 32]),
                ("wrong", [3_u8; 32]),
            ],
        )
        .expect("fixture keys should validate")
    }

    fn version_context(version_id: SecretVersionId) -> VersionContext {
        VersionContext {
            owner: SecretOwner::Project(ProjectId::new()),
            secret_id: SecretId::new(),
            version_id,
            sequence: 1,
            media_type: String::from("application/octet-stream"),
        }
    }

    fn sealed() -> (
        EncryptedStore<LocalKeyProvider>,
        VersionContext,
        EncryptedSecretVersion,
    ) {
        let store = EncryptedStore::new(provider("key/v1"));
        let context = version_context(SecretVersionId::new());
        let plaintext = SecretValue::new(SENTINEL).expect("sentinel should validate");
        let encrypted = store
            .seal(&context, &plaintext)
            .expect("encryption should succeed");
        (store, context, encrypted)
    }

    #[test]
    fn envelope_round_trip_and_nonce_uniqueness() {
        let (store, context, first) = sealed();
        let plaintext = SecretValue::new(SENTINEL).expect("sentinel should validate");
        let second = store
            .seal(&version_context(SecretVersionId::new()), &plaintext)
            .expect("second encryption should succeed");
        assert_ne!(first.data_nonce, second.data_nonce);
        assert_ne!(first.wrap_nonce, second.wrap_nonce);
        assert_ne!(first.ciphertext, second.ciphertext);
        assert!(
            !first
                .ciphertext
                .windows(SENTINEL.len())
                .any(|v| v == SENTINEL)
        );
        assert!(
            !first
                .wrapped_data_key
                .windows(SENTINEL.len())
                .any(|value| value == SENTINEL)
        );

        let resolved = store
            .resolve(&context, &first)
            .expect("authorized exact context should resolve");
        assert_eq!(resolved.expose(), SENTINEL);
    }

    #[test]
    fn ciphertext_nonce_and_associated_data_tampering_fail() {
        let (store, context, encrypted) = sealed();
        let mut mutations = Vec::new();
        let mut ciphertext = encrypted.clone();
        ciphertext.ciphertext[0] ^= 1;
        mutations.push(ciphertext);
        let mut data_nonce = encrypted.clone();
        data_nonce.data_nonce[0] ^= 1;
        mutations.push(data_nonce);
        let mut wrapped_key = encrypted.clone();
        wrapped_key.wrapped_data_key[0] ^= 1;
        mutations.push(wrapped_key);
        let mut wrap_nonce = encrypted.clone();
        wrap_nonce.wrap_nonce[0] ^= 1;
        mutations.push(wrap_nonce);
        let mut hash = encrypted;
        hash.associated_data_hash[0] ^= 1;
        mutations.push(hash);
        for mutation in mutations {
            assert!(store.resolve(&context, &mutation).is_err());
        }
        let mut wrong_context = context.clone();
        wrong_context.sequence = 2;
        assert!(
            store
                .resolve(&wrong_context, &mutations_fixture(&store, &context))
                .is_err()
        );
    }

    fn mutations_fixture(
        store: &EncryptedStore<LocalKeyProvider>,
        context: &VersionContext,
    ) -> EncryptedSecretVersion {
        store
            .seal(
                context,
                &SecretValue::new(SENTINEL).expect("sentinel should validate"),
            )
            .expect("fixture encryption should work")
    }

    #[test]
    fn wrong_and_unavailable_keys_fail_closed() {
        let (_store, context, mut encrypted) = sealed();
        encrypted.key_reference = String::from("wrong");
        let wrong = EncryptedStore::new(provider("key/v1"));
        assert!(matches!(
            wrong.resolve(&context, &encrypted),
            Err(SecretStoreError::Authentication)
        ));

        let unavailable = LocalKeyProvider::new("key/v2", [("key/v2", [2_u8; 32])])
            .expect("remaining key should validate");
        let unavailable = EncryptedStore::new(unavailable);
        encrypted.key_reference = String::from("key/v1");
        assert!(matches!(
            unavailable.resolve(&context, &encrypted),
            Err(SecretStoreError::UnavailableKey)
        ));
    }

    #[test]
    fn host_key_rotation_affects_only_later_versions_and_backups_restore() {
        let mut keys = provider("key/v1");
        let first_store = EncryptedStore::new(provider("key/v1"));
        let first_context = version_context(SecretVersionId::new());
        let first = first_store
            .seal(
                &first_context,
                &SecretValue::new(SENTINEL).expect("sentinel should validate"),
            )
            .expect("first encryption should work");

        keys.rotate_active_key("key/v2")
            .expect("known key should activate");
        let rotated = EncryptedStore::new(keys);
        let second_context = version_context(SecretVersionId::new());
        let second = rotated
            .seal(
                &second_context,
                &SecretValue::new(SENTINEL).expect("sentinel should validate"),
            )
            .expect("rotated encryption should work");
        assert_eq!(first.key_reference, "key/v1");
        assert_eq!(second.key_reference, "key/v2");
        assert_eq!(
            rotated
                .resolve(&first_context, &first)
                .expect("retained backup key should decrypt")
                .expose(),
            SENTINEL
        );
        assert_eq!(
            rotated
                .resolve(&second_context, &second)
                .expect("active key should decrypt")
                .expose(),
            SENTINEL
        );
    }

    #[test]
    fn formatted_records_and_errors_never_expose_plaintext_or_keys() {
        let (_store, _context, encrypted) = sealed();
        let error = SecretStoreError::Authentication;
        for rendered in [
            format!("{encrypted:?}"),
            format!("{error:?}"),
            error.to_string(),
        ] {
            assert!(!rendered.contains(std::str::from_utf8(SENTINEL).expect("sentinel is UTF-8")));
            assert!(!rendered.contains(&"01".repeat(32)));
        }
    }
}
