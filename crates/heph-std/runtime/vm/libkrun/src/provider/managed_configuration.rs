use super::{LibkrunConfig, LibkrunProvider, VmError, VmProvider};
use vm_trait::VmProviderOwnerScope;

impl LibkrunProvider {
    /// Verifies the complete retained configuration and current managed owner.
    ///
    /// Reads existing ownership metadata through [`VmProvider::owner_scope`];
    /// does not allocate a provider or initialize or repair ownership.
    ///
    /// # Errors
    ///
    /// Rejects any configuration mismatch, unmanaged or stale ownership, or a
    /// different expected owner scope.
    pub fn verify_managed_configuration(
        &self,
        expected: &LibkrunConfig,
        expected_owner: &VmProviderOwnerScope,
    ) -> Result<(), VmError> {
        if self.inner.config.as_ref() != expected {
            return Err(VmError::InvalidState(
                "managed VM provider configuration differs",
            ));
        }
        if <Self as VmProvider>::owner_scope(self)?.ne(expected_owner) {
            return Err(VmError::InvalidState(
                "managed VM provider owner scope differs",
            ));
        }
        Ok(())
    }
}
