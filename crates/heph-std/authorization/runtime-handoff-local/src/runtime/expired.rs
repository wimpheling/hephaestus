//! Authenticated terminal deletion using this store's retained root and key.

use super::{ALGORITHM, EncryptedFileHandoffStore, associated_data};
use crate::envelope::ExpiredEnvelope;
use aes_gcm::aead::{Aead, Payload};
use capability_domain::{RuntimeCredential, RuntimeSessionStatus};
use runtime_authority::{ExpiredRuntimeHandoff, RuntimeAuthorityError};
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

impl EncryptedFileHandoffStore {
    pub(super) fn destroy_expired_original(
        &self,
        original: &ExpiredRuntimeHandoff,
    ) -> Result<(), RuntimeAuthorityError> {
        let session = original.session();
        if session.status != RuntimeSessionStatus::Expired
            || session.expires_at <= session.issued_at
            || original.observed_at() < session.expires_at
        {
            return Err(RuntimeAuthorityError::IdentityMismatch);
        }
        // Caller must freshly qualify this closed original through its retained
        // issuer. This local operation cannot establish PostgreSQL provenance.
        let name = crate::pinned::Name::runtime(session.id, session.generation);
        let leaf = self
            .root
            .acquire(&self.directory, name)
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let Some(leaf) = leaf else {
            return Ok(());
        };
        let bytes = leaf
            .bytes()
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        let envelope: ExpiredEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?;
        if envelope.algorithm != ALGORITHM
            || envelope.expires_at_unix_nanos != session.expires_at.unix_timestamp_nanos()
        {
            return Err(RuntimeAuthorityError::HandoffUnavailable);
        }
        let aad = associated_data(
            session.id,
            session.generation,
            envelope.expires_at_unix_nanos,
        );
        let plaintext = Zeroizing::new(
            self.cipher()?
                .decrypt(
                    (&envelope.nonce).into(),
                    Payload {
                        msg: &envelope.ciphertext,
                        aad: &aad,
                    },
                )
                .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?,
        );
        let secret = Zeroizing::new(
            <[u8; 32]>::try_from(plaintext.as_slice())
                .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)?,
        );
        let credential = RuntimeCredential::from_secret(*secret);
        let verified =
            original
                .credential_hash()
                .verifies(&credential, session.id, session.generation);
        drop(credential);
        drop(secret);
        drop(plaintext);
        if !verified {
            return Err(RuntimeAuthorityError::HandoffUnavailable);
        }
        // Never call the legacy blind destroy or a directory-wide expiry purge.
        self.root
            .unlink(&self.directory, &leaf)
            .map_err(|_| RuntimeAuthorityError::HandoffUnavailable)
    }
}
