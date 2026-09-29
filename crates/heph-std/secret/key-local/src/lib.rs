//! Host-loaded versioned key provider for the encrypted secret store.

use secret_store::{KeyProvider, SecretStoreError};
use std::{
    collections::BTreeMap,
    error::Error,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::Path,
};
use zeroize::Zeroizing;

/// In-process provider for service-owned host key material.
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
    /// keys for restore and decryption.
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

    /// Removes a retired key. Existing versions under that key then fail
    /// closed until a backup restores it.
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

    /// Loads owner-only raw key files from a service-owned directory.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory or files are not private regular
    /// files owned by the current service, or when key material is malformed.
    pub fn from_directory(
        directory: &Path,
        active_reference: impl Into<String>,
    ) -> Result<Self, Box<dyn Error>> {
        if !directory.is_absolute() {
            return Err("HEPHAESTUS_SECRET_KEY_DIRECTORY must be absolute".into());
        }
        let metadata = std::fs::symlink_metadata(directory)?;
        let process_uid = std::fs::metadata("/proc/self")?.uid();
        if metadata.file_type().is_symlink()
            || !metadata.is_dir()
            || metadata.uid() != process_uid
            || metadata.permissions().mode() & 0o777 != 0o700
        {
            return Err("secret key directory must be service-owned mode 0700".into());
        }
        let mut paths = std::fs::read_dir(directory)?
            .map(|entry| entry.map(|value| value.path()))
            .collect::<Result<Vec<_>, _>>()?;
        paths.sort();
        let mut keys = Vec::with_capacity(paths.len());
        for key_path in paths {
            let key_metadata = std::fs::symlink_metadata(&key_path)?;
            if key_metadata.file_type().is_symlink()
                || !key_metadata.is_file()
                || key_metadata.uid() != process_uid
                || key_metadata.permissions().mode() & 0o777 != 0o400
            {
                return Err("secret key files must be service-owned regular mode 0400".into());
            }
            let reference = key_path
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or("secret key reference filename must be UTF-8")?
                .to_owned();
            let bytes = Zeroizing::new(std::fs::read(&key_path)?);
            if bytes.len() != 32 {
                return Err("secret key files must contain exactly 32 raw bytes".into());
            }
            keys.push((reference, bytes));
        }
        Self::new(active_reference, keys).map_err(Into::into)
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

#[cfg(test)]
mod tests {
    use super::LocalKeyProvider;
    use secret_store::KeyProvider;
    use std::{fs, os::unix::fs::PermissionsExt};

    #[test]
    fn loads_a_strict_multi_version_key_directory() {
        let temporary = tempfile::tempdir().expect("key directory parent");
        let directory = temporary.path().join("keys");
        fs::create_dir(&directory).expect("key directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .expect("key directory mode");
        for (reference, byte) in [("local-v1", 1_u8), ("local-v2", 2_u8)] {
            let path = directory.join(reference);
            fs::write(&path, [byte; 32]).expect("key file");
            fs::set_permissions(path, fs::Permissions::from_mode(0o400)).expect("key file mode");
        }
        let provider =
            LocalKeyProvider::from_directory(&directory, "local-v2").expect("valid key ring");
        assert_eq!(
            provider.active_key_reference().expect("active key"),
            "local-v2"
        );
        assert!(provider.key("local-v1").is_ok());

        fs::set_permissions(
            directory.join("local-v1"),
            fs::Permissions::from_mode(0o600),
        )
        .expect("unsafe key mode");
        assert!(LocalKeyProvider::from_directory(&directory, "local-v2").is_err());
    }
}
