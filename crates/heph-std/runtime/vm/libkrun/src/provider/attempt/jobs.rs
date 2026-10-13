//! Serialized owning jobs: cancellation only abandons a result receiver.
use super::{
    Inner,
    binding::{Cleanup, Start},
    retained_error,
};
use crate::provider::{
    helpers::{provider_error, unavailable_error},
    launch::{LaunchState, PendingLaunch},
};
use std::sync::Arc;
use vm_trait::{VmError, VmInstance};

pub(super) async fn provision(owner: &Inner) -> Result<Arc<dyn VmInstance>, VmError> {
    let _operation = owner.operation.lock().await;
    match owner.binding.start()? {
        Start::Cached(result) => result.map_err(retained_error),
        Start::Spec(spec) => {
            let result =
                super::super::provision::run(&owner.provider, *spec, Some(&owner.binding)).await;
            owner.binding.finish(result)
        }
    }
}

pub(super) async fn cleanup(owner: &Inner) -> Result<(), VmError> {
    let _operation = owner.operation.lock().await;
    let result = match owner.binding.cleanup_target() {
        Ok(Cleanup::NoEffects) => Ok(()),
        Ok(Cleanup::Pending { pending, instance }) => {
            cleanup_launch(owner, pending, instance).await
        }
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => {
            owner.binding.cleaned();
            Ok(())
        }
        Err(error) => Err(owner.binding.error(error)),
    }
}

// The actual native task, not a watch marker, certifies its owner has settled.
// A timeout polls by mutable reference and leaves this SAME handle retained.
pub(super) struct LaunchJob {
    handle: tokio::task::JoinHandle<()>,
    outcome: Option<Result<(), Arc<VmError>>>,
}
impl LaunchJob {
    pub(super) const fn new(handle: tokio::task::JoinHandle<()>) -> Self {
        Self {
            handle,
            outcome: None,
        }
    }
    async fn join(&mut self, bound: std::time::Duration) -> Result<(), VmError> {
        if let Some(outcome) = &self.outcome {
            return outcome.clone().map_err(retained_error);
        }
        let result = match tokio::time::timeout(bound, &mut self.handle).await {
            Ok(result) => {
                result.map_err(|error| Arc::new(provider_error("original launch owner", error)))
            }
            Err(_) => {
                return Err(unavailable_error(
                    "original launch owner",
                    "original owner remains unsettled",
                ));
            }
        };
        self.outcome = Some(result.clone());
        result.map_err(retained_error)
    }
}
async fn cleanup_launch(
    owner: &Inner,
    pending: Arc<PendingLaunch>,
    instance: Option<Arc<dyn VmInstance>>,
) -> Result<(), VmError> {
    let job = owner.binding.launch_owner()?;
    // Initiate SAME original drain/retry before joining: Held owners wait for
    // this retry and must not be joined first. A new Held failure stays held.
    let retry = matches!(*pending.state.borrow(), LaunchState::Held(_));
    tokio::time::timeout(
        owner.provider.inner.config.startup_timeout,
        pending.drain_cleanup(retry),
    )
    .await
    .map_err(|_| {
        unavailable_error(
            "original launch cleanup",
            "original owner remains unsettled",
        )
    })??;
    job.lock()
        .await
        .join(owner.provider.inner.config.startup_timeout)
        .await?;
    let final_state = pending.state.borrow().clone();
    match final_state {
        LaunchState::Cleaned => Ok(()),
        LaunchState::HandedOff => {
            let instance = instance.ok_or(VmError::InvalidState(
                "original handed-off instance missing",
            ))?;
            instance.destroy().await
        }
        LaunchState::Held(failure) => Err(provider_error("original launch cleanup", failure)),
        LaunchState::Working | LaunchState::Handoff => Err(VmError::InvalidState(
            "original pending cleanup is not positively settled",
        )),
    }
}
