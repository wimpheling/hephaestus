//! Crash-recoverable encrypted handoff for separate runtime Git credentials.

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, AeadCore, OsRng, Payload},
};
use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
use runtime_git_authority::{
    RuntimeGitAuthorityError, RuntimeGitCredential, RuntimeGitHandoffStore,
};
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

const ALGORITHM: &str = "AES-256-GCM/hephaestus-runtime-git-handoff-v1";
const EXTENSION: &str = "git-handoff";

/// Filesystem-backed encrypted runtime Git handoff on one trusted VM host.
pub struct EncryptedFileRuntimeGitHandoffStore {
    directory: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl EncryptedFileRuntimeGitHandoffStore {
    /// Initializes host-private storage under an existing handoff root.
    ///
    /// # Errors
    ///
    /// Fails when the directory cannot be created or constrained to owner-only
    /// access.
    pub fn new(
        directory: impl Into<PathBuf>,
        key: [u8; 32],
    ) -> Result<Self, RuntimeGitAuthorityError> {
        let directory = directory.into();
        fs::create_dir_all(&directory).map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
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
            "{}-{}.{}",
            session_id.as_uuid().hyphenated(),
            generation.get(),
            EXTENSION
        ))
    }

    fn cipher(&self) -> Result<Aes256Gcm, RuntimeGitAuthorityError> {
        Aes256Gcm::new_from_slice(self.key.as_ref())
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)
    }

    fn decode(
        &self,
        path: &Path,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        let bytes = fs::read(path).map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        let envelope: StoredEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        if envelope.algorithm != ALGORITHM
            || envelope.expires_at_unix_nanos <= now.unix_timestamp_nanos()
        {
            return Err(RuntimeGitAuthorityError::HandoffUnavailable);
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
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        let secret = plaintext
            .try_into()
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        Ok(RuntimeGitCredential::from_secret(secret))
    }
}

impl RuntimeGitHandoffStore for EncryptedFileRuntimeGitHandoffStore {
    fn create(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        expires_at: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
        let target = self.path(session_id, generation);
        if target.exists() {
            return Err(RuntimeGitAuthorityError::HandoffExists);
        }

        let secret_key = Aes256Gcm::generate_key(&mut OsRng);
        let secret: [u8; 32] = secret_key.into();
        let credential = RuntimeGitCredential::from_secret(secret);
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
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        let bytes = serde_json::to_vec(&StoredEnvelope {
            algorithm: String::from(ALGORITHM),
            nonce: nonce.into(),
            ciphertext,
            expires_at_unix_nanos,
        })
        .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        let temporary = self.directory.join(format!(
            ".{}-{}.git.tmp",
            session_id.as_uuid().hyphenated(),
            Uuid::new_v4().hyphenated()
        ));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
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
                Err(RuntimeGitAuthorityError::HandoffExists)
            }
            Err(_) => Err(RuntimeGitAuthorityError::HandoffUnavailable),
        }
    }

    fn open(
        &self,
        session_id: RuntimeSessionId,
        generation: RuntimeCredentialGeneration,
        now: OffsetDateTime,
    ) -> Result<RuntimeGitCredential, RuntimeGitAuthorityError> {
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
    ) -> Result<(), RuntimeGitAuthorityError> {
        match fs::remove_file(self.path(session_id, generation)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(RuntimeGitAuthorityError::HandoffUnavailable),
        }
    }

    fn purge_expired(&self, now: OffsetDateTime) -> Result<u64, RuntimeGitAuthorityError> {
        let mut removed = 0_u64;
        for entry in fs::read_dir(&self.directory)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?
        {
            let entry = entry.map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
            if entry.path().extension().and_then(|value| value.to_str()) != Some(EXTENSION) {
                continue;
            }
            let bytes =
                fs::read(entry.path()).map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
            let envelope: StoredEnvelope = serde_json::from_slice(&bytes)
                .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
            if envelope.expires_at_unix_nanos <= now.unix_timestamp_nanos() {
                fs::remove_file(entry.path())
                    .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
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
    use super::EncryptedFileRuntimeGitHandoffStore;
    use capability_domain::{RuntimeCredentialGeneration, RuntimeSessionId};
    use runtime_git_authority::RuntimeGitHandoffStore;
    use time::{Duration, OffsetDateTime};

    #[test]
    fn redelivery_is_exact_and_uses_separate_envelope_namespace() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let session_id = RuntimeSessionId::new();
        let generation = RuntimeCredentialGeneration::INITIAL;
        let now = OffsetDateTime::UNIX_EPOCH + Duration::hours(1);
        let store = EncryptedFileRuntimeGitHandoffStore::new(directory.path(), [0x71; 32])
            .expect("Git handoff store");
        let first = store
            .create(session_id, generation, now + Duration::minutes(5))
            .expect("create Git handoff");
        let reopened = store
            .open(session_id, generation, now)
            .expect("reopen Git handoff");
        assert_eq!(first.expose(), reopened.expose());
        assert!(
            directory
                .path()
                .join(format!(
                    "{}-1.git-handoff",
                    session_id.as_uuid().hyphenated()
                ))
                .is_file()
        );
    }
}
