//! Bounded retryable cleanup for one persistent gateway service identity.

use std::{sync::Arc, time::Duration};

use tokio::time;
use vm_trait::{VmId, VmInstance, VmProvider};

use crate::{
    GatewayServiceIdentity, GatewayServiceLaunchResolver, service_instance::MAX_SHUTDOWN_TIMEOUT,
};

/// Redacted errors from one bounded physical cleanup attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GatewayServiceCleanupError {
    /// The identity, VM ID, or timeout was invalid.
    #[error("invalid gateway service cleanup input")]
    InvalidInput,
    /// VM teardown did not complete within the bounded attempt.
    #[error("gateway service VM teardown is incomplete")]
    VmTeardownIncomplete,
    /// Materializer cleanup did not complete after VM teardown.
    #[error("gateway service materializer cleanup is incomplete")]
    MaterializerCleanupIncomplete,
}

/// Retryable physical cleanup state for one exact service identity.
pub struct GatewayServiceCleanup {
    identity: GatewayServiceIdentity,
    vm: Option<Arc<dyn VmInstance>>,
    vm_teardown_confirmed: bool,
    materializer_cleanup_confirmed: bool,
    timeout: Duration,
}

impl GatewayServiceCleanup {
    /// Creates cleanup state for a retained VM or a deterministic orphan ID.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupError::InvalidInput`] for nil identity
    /// fields, an invalid deterministic VM ID, or an out-of-bounds timeout.
    pub fn new(
        identity: GatewayServiceIdentity,
        vm: Option<Arc<dyn VmInstance>>,
        timeout: Duration,
    ) -> Result<Self, GatewayServiceCleanupError> {
        if identity.instance_id.is_nil()
            || identity.gateway_id.is_nil()
            || identity.revision_id.is_nil()
            || timeout.is_zero()
            || timeout > MAX_SHUTDOWN_TIMEOUT
        {
            return Err(GatewayServiceCleanupError::InvalidInput);
        }
        if vm
            .as_ref()
            .is_some_and(|vm| vm.id() != &Self::vm_id(identity))
        {
            return Err(GatewayServiceCleanupError::InvalidInput);
        }
        Ok(Self {
            identity,
            vm,
            vm_teardown_confirmed: false,
            materializer_cleanup_confirmed: false,
            timeout,
        })
    }

    /// Transfers caller-proven physical cleanup progress into retry state.
    ///
    /// The caller must have already confirmed both provider teardown and exact
    /// materializer cleanup.  This constructor never infers completion from a
    /// missing VM handle.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupError::InvalidInput`] when the identity
    /// or timeout violates the cleanup bounds.
    pub fn from_confirmed_physical(
        identity: GatewayServiceIdentity,
        timeout: Duration,
    ) -> Result<Self, GatewayServiceCleanupError> {
        let mut cleanup = Self::new(identity, None, timeout)?;
        cleanup.vm_teardown_confirmed = true;
        cleanup.materializer_cleanup_confirmed = true;
        Ok(cleanup)
    }

    /// Transfers independently confirmed teardown and materializer progress
    /// into retry state without inferring either flag from a missing handle.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayServiceCleanupError::InvalidInput`] when a retained
    /// VM is marked torn down or materializer cleanup is marked complete
    /// before VM teardown.
    pub fn from_progress(
        identity: GatewayServiceIdentity,
        vm: Option<Arc<dyn VmInstance>>,
        vm_teardown_confirmed: bool,
        materializer_cleanup_confirmed: bool,
        timeout: Duration,
    ) -> Result<Self, GatewayServiceCleanupError> {
        if (vm.is_some() && vm_teardown_confirmed)
            || (materializer_cleanup_confirmed && !vm_teardown_confirmed)
        {
            return Err(GatewayServiceCleanupError::InvalidInput);
        }
        let mut cleanup = Self::new(identity, vm, timeout)?;
        cleanup.vm_teardown_confirmed = vm_teardown_confirmed;
        cleanup.materializer_cleanup_confirmed = materializer_cleanup_confirmed;
        if cleanup.vm_teardown_confirmed {
            cleanup.vm = None;
        }
        Ok(cleanup)
    }

    /// Returns the exact service identity owned by this cleanup state.
    #[must_use]
    pub const fn identity(&self) -> GatewayServiceIdentity {
        self.identity
    }

    /// Returns whether provider teardown has been confirmed.
    #[must_use]
    pub const fn vm_teardown_confirmed(&self) -> bool {
        self.vm_teardown_confirmed
    }

    /// Returns whether materializer cleanup has been confirmed.
    #[must_use]
    pub const fn materializer_cleanup_confirmed(&self) -> bool {
        self.materializer_cleanup_confirmed
    }

    /// Returns the retained VM handle, when teardown still needs retrying.
    #[must_use]
    pub fn retained_vm(&self) -> Option<Arc<dyn VmInstance>> {
        self.vm.clone()
    }

    /// Attempts provider teardown and then exact materializer cleanup.
    ///
    /// The caller owns this state and may retry the same operation while
    /// retaining it alongside lease monitoring. No task is spawned here.
    ///
    /// # Errors
    ///
    /// Returns a redacted error while preserving the state needed for retry.
    pub async fn attempt(
        &mut self,
        provider: &dyn VmProvider,
        resolver: &dyn GatewayServiceLaunchResolver,
    ) -> Result<(), GatewayServiceCleanupError> {
        if !self.vm_teardown_confirmed {
            let teardown = if let Some(vm) = self.vm.as_ref() {
                time::timeout(self.timeout, vm.destroy())
                    .await
                    .is_ok_and(|result| result.is_ok())
            } else {
                time::timeout(
                    self.timeout,
                    provider.cleanup_orphan(&Self::vm_id(self.identity)),
                )
                .await
                .is_ok_and(|result| result.is_ok())
            };
            if !teardown {
                return Err(GatewayServiceCleanupError::VmTeardownIncomplete);
            }
            self.vm = None;
            self.vm_teardown_confirmed = true;
        }
        if !self.materializer_cleanup_confirmed {
            time::timeout(self.timeout, resolver.cleanup_service_launch(self.identity))
                .await
                .is_ok_and(|result| result.is_ok())
                .then_some(())
                .ok_or(GatewayServiceCleanupError::MaterializerCleanupIncomplete)?;
            self.materializer_cleanup_confirmed = true;
        }
        Ok(())
    }

    fn vm_id(identity: GatewayServiceIdentity) -> VmId {
        VmId(format!("gateway-service-{}", identity.instance_id))
    }
}

#[cfg(test)]
#[path = "service_cleanup/tests.rs"]
mod tests;
