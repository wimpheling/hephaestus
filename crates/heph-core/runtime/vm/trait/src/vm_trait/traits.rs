use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::broadcast;

use super::{
    BoxedPrivateServiceConnection, PrivateHttpRequest, PrivateHttpResponse, StopMode, VmError,
    VmEvent, VmExit, VmId, VmSpec,
};

/// Provisions virtual machines using a particular backend.
///
/// Provisioning allocates the resources described by a [`VmSpec`] but does not
/// start the guest. Call [`VmInstance::start`] explicitly after provisioning.
#[async_trait]
pub trait VmProvider: Send + Sync + 'static {
    /// Returns the stable name of this provider implementation.
    fn name(&self) -> &'static str;

    /// Allocates a stopped VM from `spec`.
    ///
    /// A failed provisioning attempt must detach all caller-owned resources
    /// and clean up provider-owned resources before returning. It must preserve
    /// every caller-owned path in `spec`.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::AlreadyExists`] when the identifier is already in
    /// use, or another [`VmError`] when resource allocation fails.
    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError>;

    /// Confirms that resources belonging to an abandoned VM identifier are
    /// destroyed after a supervisor restart.
    ///
    /// This operation is idempotent. It may terminate orphaned workers and
    /// remove provider-owned runtime resources, but it must preserve every
    /// caller-owned path previously supplied through a [`VmSpec`]. Successful
    /// completion confirms that disks are detached and the identifier can be
    /// safely reused.
    ///
    /// # Errors
    ///
    /// Returns an error when complete cleanup and disk detachment cannot be
    /// confirmed.
    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError>;
}

/// A provisioned VM with an explicit lifecycle.
///
/// Instances are returned in a provisioned state. They are one-shot, must be
/// started explicitly, and must be destroyed when their resources are no
/// longer required. Dropping a handle does not release provider resources.
#[async_trait]
pub trait VmInstance: Send + Sync + 'static {
    /// Returns the stable identifier assigned during provisioning.
    fn id(&self) -> &VmId;

    /// Starts the guest, or joins an in-progress start.
    ///
    /// This operation is idempotent while the VM is starting or running. An
    /// exited VM cannot be restarted.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if the instance cannot be started
    /// from its current state, or another [`VmError`] if startup fails.
    async fn start(&self) -> Result<(), VmError>;

    /// Stops the guest using the requested strategy.
    ///
    /// This operation is idempotent. A provisioned, stopped, or exited VM
    /// returns successfully. Graceful stop requests guest cancellation and
    /// force-stops the VM after its timeout.
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if the instance cannot be stopped
    /// from its current state, or another [`VmError`] if shutdown fails.
    async fn stop(&self, mode: StopMode) -> Result<(), VmError>;

    /// Waits until the guest exits and returns its cached termination status.
    ///
    /// Any number of callers may wait concurrently or after exit. Every
    /// successful caller receives the same [`VmExit`]. A VM destroyed before
    /// startup returns [`VmError::Destroyed`].
    ///
    /// # Errors
    ///
    /// Returns [`VmError::InvalidState`] if waiting is not valid in the
    /// instance's current state, or another [`VmError`] if monitoring fails.
    async fn wait(&self) -> Result<VmExit, VmError>;

    /// Invokes a bounded HTTP handler through the provider's private
    /// host-to-guest transport.
    ///
    /// Providers that have not implemented the authenticated handler protocol
    /// must fail closed with [`VmError::Unsupported`].  This method never
    /// creates a guest listener or broadens [`crate::NetworkMode`].
    ///
    /// # Errors
    ///
    /// Returns an error if the VM is not running, private handler transport is
    /// unsupported, or the provider cannot complete the bounded exchange.
    async fn invoke_private_http(
        &self,
        _request: PrivateHttpRequest,
    ) -> Result<PrivateHttpResponse, VmError> {
        Err(VmError::Unsupported {
            feature: "private HTTP handler transport".to_owned(),
            provider: "unspecified".to_owned(),
        })
    }

    /// Opens one full-duplex connection to a declared long-lived guest HTTP
    /// service. Providers without the private service transport fail closed.
    async fn open_private_service_connection(
        &self,
    ) -> Result<BoxedPrivateServiceConnection, VmError> {
        Err(VmError::Unsupported {
            feature: "private HTTP service transport".to_owned(),
            provider: "unspecified".to_owned(),
        })
    }

    /// Subscribes to best-effort live lifecycle and log events.
    ///
    /// The returned receiver reports lag and channel closure using Tokio's
    /// broadcast receiver errors. Events emitted before subscription are not
    /// replayed.
    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent>;

    /// Releases all resources owned by the instance.
    ///
    /// This operation is idempotent and force-terminates a running VM. If an
    /// exit status was already cached, destroying the VM does not discard it.
    /// Successful completion confirms that all disks and mounts are detached.
    /// Caller-owned root, disk, and mount backing paths are always preserved;
    /// only provider-owned runtime resources are removed.
    ///
    /// # Errors
    ///
    /// Returns a [`VmError`] if complete cleanup cannot be confirmed.
    async fn destroy(&self) -> Result<(), VmError>;
}
