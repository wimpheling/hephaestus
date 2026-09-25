//! Parent-owned preparation of one persistent gateway service instance.
//!
//! The parent must insert [`ServicePreparation::run`] into its own task set
//! and join it. Dropping [`ServicePreparationHandle`] requests cancellation;
//! it does not abandon an in-flight resolver or provider future. Forced
//! process shutdown relies on the persisted ownership ledger.

use std::fmt;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;
use vm_trait::{VmId, VmInstance, VmProvider};

use crate::{
    GatewayServiceIdentity, GatewayServiceLaunch, GatewayServiceLaunchRequest,
    GatewayServiceLaunchResolver,
};

/// A stopped VM paired with the exact immutable launch it was prepared for.
pub struct PreparedGatewayService {
    /// Immutable service launch selected by the durable resolver.
    pub launch: GatewayServiceLaunch,
    /// Stopped VM owned by the parent supervisor.
    pub vm: Arc<dyn VmInstance>,
}

impl fmt::Debug for PreparedGatewayService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedGatewayService")
            .field("identity", &self.launch.identity)
            .field("vm_id", self.vm.id())
            .finish()
    }
}

/// Redacted reason for a preparation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServicePreparationFailureReason {
    /// The resolver could not produce the exact immutable launch.
    Resolution,
    /// The resolver returned an invalid identity or VM specification.
    InvalidLaunch,
    /// Provider provisioning returned an error.
    Provisioning,
    /// The parent requested cancellation.
    Cancelled,
    /// Provider or materializer cleanup could not be confirmed.
    CleanupIncomplete,
}

/// Preparation failure retaining every live ownership handle required for retry.
pub struct ServicePreparationFailure {
    /// Exact service identity owned by this preparation attempt.
    pub identity: GatewayServiceIdentity,
    /// Redacted failure classification.
    pub reason: ServicePreparationFailureReason,
    /// Returned VM retained when destroy did not complete.
    pub vm: Option<Arc<dyn VmInstance>>,
    /// Whether the exact materialization may still exist and must be retained.
    pub materialization_owned: bool,
}

impl fmt::Debug for ServicePreparationFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServicePreparationFailure")
            .field("identity", &self.identity)
            .field("reason", &self.reason)
            .field("vm_present", &self.vm.is_some())
            .field("materialization_owned", &self.materialization_owned)
            .finish()
    }
}

/// Cancellation control for one parent-owned preparation attempt.
pub struct ServicePreparationHandle {
    cancellation: CancellationToken,
}

impl ServicePreparationHandle {
    /// Requests cancellation at the next safe phase boundary.
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
}

impl Drop for ServicePreparationHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Parent-owned preparation future.
pub struct ServicePreparation {
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    request: GatewayServiceLaunchRequest,
    cancellation: CancellationToken,
}

/// Creates one parent-owned preparation future and its cancellation handle.
///
/// The parent must retain the returned handle while the future runs, keep the
/// durable ownership heartbeat alive, and join [`ServicePreparation::run`].
/// It must not drop or abort the running future except during forced process
/// shutdown, where the persisted ownership ledger drives recovery.
#[must_use]
pub fn new_service_preparation(
    resolver: Arc<dyn GatewayServiceLaunchResolver>,
    provider: Arc<dyn VmProvider>,
    request: GatewayServiceLaunchRequest,
) -> (ServicePreparationHandle, ServicePreparation) {
    let cancellation = CancellationToken::new();
    let handle = ServicePreparationHandle {
        cancellation: cancellation.clone(),
    };
    let preparation = ServicePreparation {
        resolver,
        provider,
        request,
        cancellation,
    };
    (handle, preparation)
}

impl ServicePreparation {
    /// Resolves, materializes, and provisions one stopped service VM.
    ///
    /// The resolver and provider futures are always awaited to settlement. A
    /// cancellation observed after provisioning destroys the returned VM
    /// before removing its exact materialization.
    ///
    /// # Errors
    ///
    /// Returns ownership of a VM or materialization whenever cleanup is not
    /// confirmed, allowing the parent to retry in the same process.
    pub async fn run(self) -> Result<PreparedGatewayService, ServicePreparationFailure> {
        let identity = self.request.identity;
        if self.cancellation.is_cancelled() {
            return Err(failure(
                identity,
                ServicePreparationFailureReason::Cancelled,
                None,
                false,
            ));
        }

        // Deliberately await to completion: dropping this future can abandon a
        // resolver that has already prepared host-owned release state.
        let Ok(launch) = self.resolver.resolve_service_launch(self.request).await else {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::Resolution,
            )
            .await;
        };
        if !valid_launch(&launch, identity) {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::InvalidLaunch,
            )
            .await;
        }
        if self.cancellation.is_cancelled() {
            return cleanup_without_vm(
                &self.resolver,
                identity,
                ServicePreparationFailureReason::Cancelled,
            )
            .await;
        }

        // The provider future is likewise never selected away or dropped.
        let Ok(vm) = self.provider.provision(launch.spec.clone()).await else {
            return cleanup_after_provision_error(&self.resolver, &self.provider, identity).await;
        };
        if self.cancellation.is_cancelled() {
            return cleanup_after_vm(&self.resolver, identity, vm).await;
        }
        Ok(PreparedGatewayService { launch, vm })
    }
}

async fn cleanup_without_vm(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    identity: GatewayServiceIdentity,
    reason: ServicePreparationFailureReason,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    match resolver.cleanup_service_launch(identity).await {
        Ok(()) => Err(failure(identity, reason, None, false)),
        Err(_) => Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            None,
            true,
        )),
    }
}

async fn cleanup_after_provision_error(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    provider: &Arc<dyn VmProvider>,
    identity: GatewayServiceIdentity,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    let id = VmId(format!("gateway-service-{}", identity.instance_id));
    if provider.cleanup_orphan(&id).await.is_err() {
        return Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            None,
            true,
        ));
    }
    cleanup_without_vm(
        resolver,
        identity,
        ServicePreparationFailureReason::Provisioning,
    )
    .await
}

async fn cleanup_after_vm(
    resolver: &Arc<dyn GatewayServiceLaunchResolver>,
    identity: GatewayServiceIdentity,
    vm: Arc<dyn VmInstance>,
) -> Result<PreparedGatewayService, ServicePreparationFailure> {
    if vm.destroy().await.is_err() {
        return Err(failure(
            identity,
            ServicePreparationFailureReason::CleanupIncomplete,
            Some(vm),
            true,
        ));
    }
    cleanup_without_vm(
        resolver,
        identity,
        ServicePreparationFailureReason::Cancelled,
    )
    .await
}

fn valid_launch(launch: &GatewayServiceLaunch, identity: GatewayServiceIdentity) -> bool {
    let expected_id = format!("gateway-service-{}", identity.instance_id);
    launch.identity == identity
        && !identity.instance_id.is_nil()
        && !identity.gateway_id.is_nil()
        && !identity.revision_id.is_nil()
        && launch.spec.id == VmId(expected_id)
        && launch.service.validate().is_ok()
        && launch
            .spec
            .private_http_service
            .as_ref()
            .is_some_and(|service| service.loopback_port == launch.service.loopback_port)
}

fn failure(
    identity: GatewayServiceIdentity,
    reason: ServicePreparationFailureReason,
    vm: Option<Arc<dyn VmInstance>>,
    materialization_owned: bool,
) -> ServicePreparationFailure {
    ServicePreparationFailure {
        identity,
        reason,
        vm,
        materialization_owned,
    }
}

#[cfg(test)]
#[path = "service_preparation/tests.rs"]
mod tests;
