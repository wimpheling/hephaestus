//! Provider-neutral key lookup contract.

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
