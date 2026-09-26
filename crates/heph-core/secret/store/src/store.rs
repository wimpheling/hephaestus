//! Authenticated encryption and resolution of secret envelopes.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use secret_domain::{SecretValue, SecretVersionId};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::model::{ALGORITHM, append_field};
use super::{EncryptedSecretVersion, KeyProvider, SecretStoreError, VersionContext};

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
