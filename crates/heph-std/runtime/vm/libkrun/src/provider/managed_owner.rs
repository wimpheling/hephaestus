use super::{Arc, LibkrunConfig, LibkrunProvider, ProcessWorkerSpawner, VmError};
use crate::provider::ownership::{ProspectiveOwner, ProviderOwner};
use std::{error::Error, fmt};

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
    /// Late publication failure retains originals only while its returned error
    /// is held, as documented by [`ProspectiveOwnedVmBootstrap::publish`].
    pub fn bootstrap_owned(
        config: LibkrunConfig,
        scope: &vm_trait::VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        Self::prepare_owned_bootstrap(config, scope)?.publish()
    }

    /// Validates the complete host configuration without writing owner metadata.
    ///
    /// Uses the same validator as construction, including service identity,
    /// delegated cgroup, executables, allowed roots, KVM access and limits.
    /// This grants no ownership and does not load the worker's backend libraries.
    ///
    /// # Errors
    ///
    /// Rejects invalid configuration or unavailable required host resources.
    pub fn validate_configuration(config: &LibkrunConfig) -> Result<(), VmError> {
        super::validate_config(config)
    }

    /// Prepares an empty dedicated VM root without creating owner metadata.
    ///
    /// Full configuration validation precedes read-only acquisition of the
    /// original runtime and cgroup descriptors. The configured scope must be a
    /// trusted canonical nonnil UUID. Preparation grants no provision authority.
    /// Dropping it closes descriptors without publishing or removing anything.
    ///
    /// # Errors
    ///
    /// Rejects invalid configuration/scope, unsafe roots or existing resources.
    pub fn prepare_owned_bootstrap(
        config: LibkrunConfig,
        scope: &vm_trait::VmProviderOwnerScope,
    ) -> Result<ProspectiveOwnedVmBootstrap, VmError> {
        let provider = Self::construct(config, Arc::new(ProcessWorkerSpawner))?;
        let owner = ProspectiveOwner::prepare(&provider.inner.config, scope)?;
        Ok(ProspectiveOwnedVmBootstrap { provider, owner })
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

/// Read-only prospective ownership retaining the original VM root descriptors.
///
/// No provider getter or VM allocation is available before publication. This is
/// not a fence against same-UID writers or an atomic multi-filesystem commit.
#[must_use]
pub struct ProspectiveOwnedVmBootstrap {
    provider: LibkrunProvider,
    owner: ProspectiveOwner,
}

impl ProspectiveOwnedVmBootstrap {
    /// Revalidates and publishes ownership in the original pinned runtime root.
    ///
    /// Rechecks full configuration, root identities and dedicated-root shape
    /// before exclusive metadata creation, using the existing owner validation.
    ///
    /// # Errors
    ///
    /// Early read-only failures return the original error. Once metadata creation
    /// is attempted,
    /// `bootstrap_publication_held` retains the original descriptors and any
    /// acquired metadata locks in its private error source. Retain the actual
    /// error when propagating failure: dropping or converting it to text closes
    /// those handles and proves neither cleanup nor recovery. Written metadata
    /// remains; no rollback, deletion or automatic adoption is attempted.
    pub fn publish(self) -> Result<LibkrunProvider, VmError> {
        self.publish_with(|| Ok(()))
    }

    #[cfg(test)]
    pub(in crate::provider) fn publish_after_reservation(
        self,
        action: impl FnOnce() -> Result<(), VmError>,
    ) -> Result<LibkrunProvider, VmError> {
        self.publish_with(action)
    }

    fn publish_with(
        mut self,
        after_reservation: impl FnOnce() -> Result<(), VmError>,
    ) -> Result<LibkrunProvider, VmError> {
        LibkrunProvider::validate_configuration(&self.provider.inner.config)?;
        let owner = match self
            .owner
            .publish(&self.provider.inner.config, after_reservation)
        {
            Ok(owner) => owner,
            Err(cause) => return Err(self.publication_failure(cause)),
        };
        if self.provider.inner.owner.set(owner).is_err() {
            return Err(
                self.publication_failure(VmError::InvalidState("VM owner is already configured"))
            );
        }
        Ok(self.provider)
    }

    fn publication_failure(self, cause: VmError) -> VmError {
        if !self.owner.publication_started() {
            return cause;
        }
        VmError::Provider {
            provider: "libkrun".into(),
            code: "bootstrap_publication_held".into(),
            source: Box::new(HeldBootstrapFailure {
                cause,
                _prepared: self,
            }),
        }
    }
}

struct HeldBootstrapFailure {
    cause: VmError,
    _prepared: ProspectiveOwnedVmBootstrap,
}

impl fmt::Debug for HeldBootstrapFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("HeldBootstrapFailure")
            .field("cause", &self.cause)
            .finish_non_exhaustive()
    }
}

impl fmt::Display for HeldBootstrapFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "VM bootstrap publication remains held: {}",
            self.cause
        )
    }
}

impl Error for HeldBootstrapFailure {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(&self.cause)
    }
}
