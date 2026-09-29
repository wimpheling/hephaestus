use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;
use vm_trait::{PrivateHttpRequest, VmInstance};

use super::validation::gateway_response;
use crate::{
    GatewayEdgeError, GatewayReleaseResolver, GatewayRequest, GatewayResponse, GatewayRouteBinding,
};

/// Runs the exact released handler revision in a short-lived isolated VM.
#[async_trait]
pub trait GatewayVmHandler: Send + Sync {
    /// Invokes private HTTP for the exact immutable binding.
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError>;
}

#[async_trait]
impl<T> GatewayVmHandler for Arc<T>
where
    T: GatewayVmHandler + ?Sized,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        (**self).invoke(route, invocation_id, request).await
    }
}

/// Starts an exact released gateway VM.  Gateway control-plane code supplies
/// the immutable release mount and capability bootstrap; this edge crate never
/// selects a release from public request data.
#[async_trait]
pub trait GatewayVmLauncher: Send + Sync {
    /// Starts and returns an isolated VM for the exact immutable route binding.
    async fn launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError>;

    /// Releases host-owned launch resources after the VM is destroyed.
    async fn cleanup(&self, _: Uuid) -> Result<(), GatewayEdgeError> {
        Ok(())
    }

    /// Confirms the exact runtime-authority bootstrap observed by the guest.
    ///
    /// Test-only launchers without runtime authority can retain the default.
    async fn acknowledge_runtime_authority(
        &self,
        _: Uuid,
        _: Uuid,
        _: u64,
    ) -> Result<(), GatewayEdgeError> {
        Ok(())
    }

    /// Whether a launched VM must acknowledge its runtime authority before it
    /// can handle the request.
    async fn requires_runtime_authority_ack(&self, _: Uuid) -> Result<bool, GatewayEdgeError> {
        Ok(false)
    }
}

/// Resolves one authoritative route to its exact immutable release launch
/// specification, including the gateway runtime-session bootstrap selected by
/// Provisions the already-resolved private VM launch specification.
#[async_trait]
pub trait GatewayRuntimeLauncher: Send + Sync {
    /// Provisions one VM without changing its exact immutable launch inputs.
    async fn provision_gateway(
        &self,
        spec: vm_trait::VmSpec,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError>;
}

/// Minimal internal gateway runtime bridge.
///
/// It separates immutable release/session selection from VM provisioning while
/// retaining `PrivateHttpVmGatewayHandler` as the only request executor.
pub struct GatewayRuntimeService<R, L> {
    releases: R,
    launcher: L,
}

impl<R, L> GatewayRuntimeService<R, L> {
    /// Creates an injected release-resolution and VM-provisioning bridge.
    #[must_use]
    pub const fn new(releases: R, launcher: L) -> Self {
        Self { releases, launcher }
    }
}

#[async_trait]
impl<R, L> GatewayVmLauncher for GatewayRuntimeService<R, L>
where
    R: GatewayReleaseResolver,
    L: GatewayRuntimeLauncher,
{
    async fn launch(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
    ) -> Result<Arc<dyn VmInstance>, GatewayEdgeError> {
        let spec = self.releases.resolve_launch(route, invocation_id).await?;
        match self.launcher.provision_gateway(spec).await {
            Ok(instance) => Ok(instance),
            Err(error) => {
                let _cleanup = self.releases.cleanup_launch(invocation_id).await;
                Err(error)
            }
        }
    }

    async fn cleanup(&self, invocation_id: Uuid) -> Result<(), GatewayEdgeError> {
        self.releases.cleanup_launch(invocation_id).await
    }

    async fn acknowledge_runtime_authority(
        &self,
        invocation_id: Uuid,
        session_id: Uuid,
        generation: u64,
    ) -> Result<(), GatewayEdgeError> {
        self.releases
            .acknowledge_runtime_authority(invocation_id, session_id, generation)
            .await
    }

    async fn requires_runtime_authority_ack(&self, _: Uuid) -> Result<bool, GatewayEdgeError> {
        Ok(true)
    }
}

/// Bridges canonical gateway HTTP to a VM provider's private host-to-guest
/// handler transport.  It does not create a guest listener or port forward.
pub struct PrivateHttpVmGatewayHandler<L> {
    launcher: Arc<L>,
}

impl<L> PrivateHttpVmGatewayHandler<L> {
    /// Creates an adapter from the release-aware VM launcher.
    #[must_use]
    pub fn new(launcher: L) -> Self {
        Self {
            launcher: Arc::new(launcher),
        }
    }
}

#[async_trait]
impl<L> GatewayVmHandler for PrivateHttpVmGatewayHandler<L>
where
    L: GatewayVmLauncher + 'static,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let instance = self.launcher.launch(route, invocation_id).await?;
        // The dispatcher enforces the public execution deadline by cancelling
        // this future.  Keep the VM lifecycle in a detached task so dropping
        // that outer future cannot leak a running microVM or its provider
        // resources.  The task always destroys the one-shot instance before
        // resolving its result.
        let launcher = Arc::clone(&self.launcher);
        let task = tokio::spawn(async move {
            let invocation = async {
                let mut events = instance.subscribe_events();
                instance
                    .start()
                    .await
                    .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                let requires_authority_ack = launcher
                    .requires_runtime_authority_ack(invocation_id)
                    .await?;
                let mut acknowledged = false;
                while let Ok(event) = events.try_recv() {
                    if let vm_trait::VmEvent::RuntimeAuthorityAcknowledged {
                        session_id,
                        generation,
                    } = event
                    {
                        launcher
                            .acknowledge_runtime_authority(invocation_id, session_id, generation)
                            .await?;
                        acknowledged = true;
                    }
                }
                if requires_authority_ack && !acknowledged {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                let response = instance
                    .invoke_private_http(PrivateHttpRequest {
                        method: request.method,
                        path_and_query: request.path_and_query,
                        headers: request.headers,
                        body: request.body,
                    })
                    .await
                    .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
                Ok::<_, GatewayEdgeError>(gateway_response(response))
            }
            .await;
            let cleanup = instance.destroy().await;
            let release_cleanup = launcher.cleanup(invocation_id).await;
            if (cleanup.is_err() || release_cleanup.is_err()) && invocation.is_ok() {
                return Err(GatewayEdgeError::HandlerUnavailable);
            }
            invocation
        });
        task.await
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
    }
}
