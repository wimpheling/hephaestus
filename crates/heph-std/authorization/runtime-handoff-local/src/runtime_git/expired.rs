//! Authenticated exact Git deletion or positively qualified exact-file absence.

use super::{ALGORITHM, EncryptedFileRuntimeGitHandoffStore, associated_data};
use crate::envelope::ExpiredEnvelope;
use aes_gcm::aead::{Aead, Payload};
use capability_domain::RuntimeSessionStatus;
use runtime_git_authority::{
    ExpiredRuntimeGitHandoff, RuntimeGitAuthorityError, RuntimeGitCredential,
};
use zeroize::Zeroizing;

#[cfg(test)]
#[path = "expired/tests.rs"]
mod tests;

impl EncryptedFileRuntimeGitHandoffStore {
    pub(super) fn destroy_expired_original(
        &self,
        original: &ExpiredRuntimeGitHandoff,
    ) -> Result<(), RuntimeGitAuthorityError> {
        let session = original.ceiling().session();
        if session.status != RuntimeSessionStatus::Expired
            || session.expires_at <= session.issued_at
            || original.ceiling().observed_at() < session.expires_at
        {
            return Err(RuntimeGitAuthorityError::IdentityMismatch);
        }
        let name = crate::pinned::Name::git(session.session, session.generation);
        let leaf = self
            .root
            .acquire(&self.directory, name)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        // Absence is comparison data positively qualified by the retained issuer,
        // never find(None). Every unexpected exact file denies without deletion.
        if original.credential().is_none() {
            return if leaf.is_none() && original.credential_hash().is_none() {
                Ok(())
            } else {
                Err(RuntimeGitAuthorityError::HandoffUnavailable)
            };
        }
        let Some(leaf) = leaf else {
            return Ok(());
        };
        let verifier = original
            .credential_hash()
            .ok_or(RuntimeGitAuthorityError::IdentityMismatch)?;
        let bytes = leaf
            .bytes()
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        let envelope: ExpiredEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?;
        if envelope.algorithm != ALGORITHM
            || envelope.expires_at_unix_nanos != session.expires_at.unix_timestamp_nanos()
        {
            return Err(RuntimeGitAuthorityError::HandoffUnavailable);
        }
        let aad = associated_data(
            session.session,
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
                .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?,
        );
        let secret = Zeroizing::new(
            <[u8; 32]>::try_from(plaintext.as_slice())
                .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)?,
        );
        let credential = RuntimeGitCredential::from_secret(*secret);
        let hash_matches = credential.storage_hash().as_bytes() == *verifier;
        drop(credential);
        drop(secret);
        drop(plaintext);
        if !hash_matches {
            return Err(RuntimeGitAuthorityError::HandoffUnavailable);
        }
        self.root
            .unlink(&self.directory, &leaf)
            .map_err(|_| RuntimeGitAuthorityError::HandoffUnavailable)
    }
}
