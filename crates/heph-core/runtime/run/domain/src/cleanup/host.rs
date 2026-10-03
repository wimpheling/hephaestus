//! Stable configured provider ownership scope, never inferred from a process.

use super::{RunCleanupError, validate_provider_identity};

/// Exact configured provider namespace and host owning a VM identity.
///
/// The namespace identifies a particular durable provider owner, such as its
/// persisted identity bound to a local root or cloud account/region, rather
/// than a provider kind or a hash of its current path. Replacing that owner
/// requires a new scope; absence from a replacement owner proves nothing about
/// the old scope. Trusted configuration supplies values before provisioning IO.
/// An HTTP caller or the current recovery process does not choose this scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunCleanupHostId {
    provider_namespace: String,
    host_id: String,
}

impl RunCleanupHostId {
    /// Constructs a bounded stable provider ownership scope.
    ///
    /// # Errors
    ///
    /// Rejects empty, excessive, or unsafe namespace and host identifiers.
    pub fn new(provider_namespace: String, host_id: String) -> Result<Self, RunCleanupError> {
        validate_provider_identity(&provider_namespace)?;
        validate_provider_identity(&host_id)?;
        Ok(Self {
            provider_namespace,
            host_id,
        })
    }

    /// Returns the stable configured provider owner namespace.
    #[must_use]
    pub fn provider_namespace(&self) -> &str {
        &self.provider_namespace
    }

    /// Returns the exact execution host, including for a zero-volume run.
    #[must_use]
    pub fn host_id(&self) -> &str {
        &self.host_id
    }
}
