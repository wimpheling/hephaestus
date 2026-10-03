use super::{Arc, LibkrunConfig, LibkrunProvider, ProcessWorkerSpawner, VmError};
use crate::provider::ownership::ProviderOwner;

impl LibkrunProvider {
    /// Bootstraps an empty dedicated managed VM root with a configured scope.
    ///
    /// This is explicit prospective authorization, never a discovery fallback.
    /// The namespace must be a canonical nonnil UUID from trusted configuration.
    /// Existing managed roots must use [`Self::open_owned`].
    ///
    /// # Errors
    ///
    /// Rejects invalid configuration, nonempty roots or conflicting ownership.
    pub fn bootstrap_owned(
        config: LibkrunConfig,
        scope: &vm_trait::VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        let provider = Self::construct(config, Arc::new(ProcessWorkerSpawner))?;
        let owner = ProviderOwner::bootstrap(&provider.inner.config, scope)?;
        provider
            .inner
            .owner
            .set(owner)
            .map_err(|_| VmError::InvalidState("VM owner is already configured"))?;
        Ok(provider)
    }

    /// Opens an existing owner using the exact trusted configured scope.
    ///
    /// Opens existing metadata only; it never initializes or repairs ownership.
    /// The lifetime supervisor lock excludes another owner until all provider
    /// clones and retained VM handles have dropped.
    ///
    /// # Errors
    ///
    /// Rejects missing, stale or contradictory scope, roots or metadata.
    pub fn open_owned(
        config: LibkrunConfig,
        expected: &vm_trait::VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        let provider = Self::construct(config, Arc::new(ProcessWorkerSpawner))?;
        let owner = ProviderOwner::open_existing(&provider.inner.config, expected)?;
        provider
            .inner
            .owner
            .set(owner)
            .map_err(|_| VmError::InvalidState("VM owner is already configured"))?;
        Ok(provider)
    }
}
