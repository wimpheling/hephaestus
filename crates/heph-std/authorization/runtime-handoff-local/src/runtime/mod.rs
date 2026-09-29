//! Crash-recoverable encrypted runtime credential handoff on a trusted host.
//!
//! Envelope filenames contain only session identity and generation. Bearer
//! bytes are authenticated-encrypted under a host-loaded key and are removed
//! after exact-generation acknowledgement, revocation, or expiry.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use capability_domain::{RuntimeCredential, RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_authority::{RuntimeAuthorityError, RuntimeHandoffStore};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};
use time::OffsetDateTime;
use uuid::Uuid;
use zeroize::Zeroizing;

const ALGORITHM: &str = "AES-256-GCM/hephaestus-runtime-handoff-v1";

/// Filesystem-backed encrypted handoff store for one trusted VM host.
pub struct EncryptedFileHandoffStore {
    directory: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl EncryptedFileHandoffStore {
    /// Initializes a host-private directory and wraps host-loaded key material.
    ///
    /// # Errors
    ///
    /// Returns [`RuntimeAuthorityError::HandoffUnavailable`] when the
    /// directory cannot be created or constrained to owner-only access.
    pub fn new(
        directory: impl Into<PathBuf>,
        key: [u8; 32],
    ) -> Result<Self, RuntimeAuthorityError> {
        let directory = directory.into();
        fs::create_dir_all(&directory).map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        Ok(Self {
            directory,
            key: Zeroizing::new(key),
        })
    }

    fn path(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> PathBuf {
        self.directory.join(format!(
            "{}-{}.handoff",
            session_id.as_uuid().hyphenated(),
            generation.get()
        ))
    }

    fn cipher(&self) -> Result<Aes256Gcm, RuntimeAuthorityError> {
        Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)
    }

    fn decode(
        &self,
        path: &Path,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        let bytes = fs::read(path).map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let envelope: StoredEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        if envelope.algorithm != ALGORITHM
            || envelope.expires_at_unix_nanos <= now.unix_timestamp_nanos()
        {
            return Err(RuntimeAuthorityError::HandoffUnavailable);
        }
        let aad = associated_data(session_id, generation, envelope.expires_at_unix_nanos);
        let plaintext = self
            .cipher()?
            .decrypt(
                (&envelope.nonce).into(),
                Payload {
                    msg: &envelope.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let secret: [u8; 32] = plaintext
            .try_into()
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        Ok(RuntimeCredential::from_secret(secret))
    }
}

impl RuntimeHandoffStore for EncryptedFileHandoffStore {
    fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        expires_at: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        let target = self.path(session_id, generation);
        if target.exists() {
            return Err(RuntimeAuthorityError::HandoffExists);
        }

        let secret_key = Aes256Gcm::generate_key(&mut OsRng);
        let secret: [u8; 32] = secret_key.into();
        let credential = RuntimeCredential::from_secret(secret);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let expires_at_unix_nanos = expires_at.unix_timestamp_nanos();
        let aad = associated_data(session_id, generation, expires_at_unix_nanos);
        let ciphertext = self
            .cipher()?
            .encrypt(
                &nonce,
                Payload {
                    msg: credential.expose(),
                    aad: &aad,
                },
            )
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let envelope = StoredEnvelope {
            algorithm: String::from(ALGORITHM),
            nonce: nonce.into(),
            ciphertext,
            expires_at_unix_nanos,
        };
        let bytes =
            serde_json::to_vec(&envelope).map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let temporary = self.directory.join(format!(
            ".{}-{}.tmp",
            session_id.as_uuid().hyphenated(),
            Uuid::new_v4().hyphenated()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let persisted = (|| {
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::hard_link(&temporary, &target)?;
            Ok::<(), std::io::Error>(())
        })();
        let _ = fs::remove_file(&temporary);
        match persisted {
            Ok(()) => Ok(credential),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Err(RuntimeAuthorityError::HandoffExists)
            }
            Err(_) => Err(RuntimeAuthorityError::HandoffUnavailable),
        }
    }

    fn open(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<RuntimeCredential, RuntimeAuthorityError> {
        self.decode(
            &self.path(session_id, generation),
            session_id,
            generation,
            now,
        )
    }

    fn destroy(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
    ) -> Result<(), RuntimeAuthorityError> {
        match fs::remove_file(self.path(session_id, generation)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(RuntimeAuthorityError::HandoffUnavailable),
        }
    }

    fn purge_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeAuthorityError> {
        let mut removed = 0_u64;
        for entry in
            fs::read_dir(&self.directory).map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?
        {
            let entry = entry.map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some("handoff") {
                continue;
            }
            let bytes =
                fs::read(entry.path()).map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
            let envelope: StoredEnvelope = serde_json::from_slice(&bytes)
                .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
            if envelope.expires_at_unix_nanos <= now.unix_timestamp_nanos() {
                fs::remove_file(entry.path())
                    .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
                removed = removed.saturating_add(1);
            }
        }
        Ok(removed)
    }
}

#[derive(Serialize, Deserialize)]
struct StoredEnvelope {
    algorithm: String,
    nonce: [u8; 12],
    ciphertext: Vec<u8>,
    expires_at_unix_nanos: i128,
}

fn associated_data(
    session_id: RuntimeSessionId,
    generation: RuntimeCredentialGeneration,
    expires_at_unix_nanos: i128,
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(ALGORITHM.len() + 16 + 8 + 16);
    aad.extend_from_slice(ALGORITHM.as_bytes());
    aad.extend_from_slice(session_id.as_uuid().as_bytes());
    aad.extend_from_slice(&generation.get().to_be_bytes());
    aad.extend_from_slice(&expires_at_unix_nanos.to_be_bytes());
    aad
}

#[cfg(test)]
mod tests {
    use super::EncryptedFileHandoffStore;
    use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
    use runtime_authority::{RuntimeAuthorityError, RuntimeHandoffStore};
    use time::{Duration, OffsetDateTime};

    #[test]
    fn exact_generation_redelivery_survives_store_recreation() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let key = [0x31; 32];
        let session_id = RuntimeSessionId::new();
        let generation = RuntimeCredentialGeneration::INITIAL;
        let now = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let expires_at = now + Duration::minutes(5);
        let first = EncryptedFileHandoffStore::new(directory.path(), key)
            .expect("handoff store")
            .create(session_id, generation, expires_at)
            .expect("create envelope");

        let reopened = EncryptedFileHandoffStore::new(directory.path(), key)
            .expect("reopened handoff store")
            .open(session_id, generation, now)
            .expect("open envelope");
        assert_eq!(first.expose(), reopened.expose());
    }

    #[test]
    fn wrong_key_expiry_and_duplicate_creation_fail_closed() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let session_id = RuntimeSessionId::new();
        let generation = RuntimeCredentialGeneration::INITIAL;
        let now = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let store =
            EncryptedFileHandoffStore::new(directory.path(), [0x41; 32]).expect("handoff store");
        store
            .create(session_id, generation, now + Duration::minutes(1))
            .expect("create envelope");
        assert_eq!(
            store
                .create(session_id, generation, now + Duration::minutes(1))
                .expect_err("duplicate must fail"),
            RuntimeAuthorityError::HandoffExists
        );
        assert_eq!(
            EncryptedFileHandoffStore::new(directory.path(), [0x42; 32])
                .expect("wrong-key store")
                .open(session_id, generation, now)
                .expect_err("wrong key must fail"),
            RuntimeAuthorityError::HandoffUnavailable
        );
        assert_eq!(
            store
                .open(session_id, generation, now + Duration::minutes(1))
                .expect_err("expired envelope must fail"),
            RuntimeAuthorityError::HandoffUnavailable
        );
        assert_eq!(
            store
                .purge_expired(now + Duration::minutes(1))
                .expect("purge"),
            1
        );
    }
}
