//! Versioned host key providers.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use super::SecretStoreError;

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
