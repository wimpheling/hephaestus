//! Opaque live-call custody; no orphan lookup or provider-wide admission change.
#[path = "attempt/binding.rs"]
mod binding;
#[path = "attempt/jobs.rs"]
mod jobs;
use super::LibkrunProvider;
use crate::provider::helpers::{provider_error, unavailable_error};
pub(super) use binding::Binding;
use std::sync::Arc;
use vm_trait::{VmError, VmInstance, VmProvisionAttempt, VmSpec};

struct Inner {
    provider: LibkrunProvider,
    binding: Binding,
    operation: tokio::sync::Mutex<()>,
}
struct Attempt(Arc<Inner>);

pub(super) fn prepare(provider: LibkrunProvider, spec: VmSpec) -> Arc<dyn VmProvisionAttempt> {
    Arc::new(Attempt(Arc::new(Inner {
        provider,
        binding: Binding::new(spec),
        operation: tokio::sync::Mutex::new(()),
    })))
}

#[cfg(test)]
// The binding is moved into the returned owner; dropping it after the move is invalid.
#[allow(clippy::significant_drop_tightening)]
pub(in super::super) fn prepare_close_on_capture(
    provider: LibkrunProvider,
    spec: VmSpec,
) -> Arc<dyn VmProvisionAttempt> {
    let binding = Binding::new(spec);
    binding.close_on_capture();
    Arc::new(Attempt(Arc::new(Inner {
        provider,
        binding,
        operation: tokio::sync::Mutex::new(()),
    })))
}

#[async_trait::async_trait]
impl VmProvisionAttempt for Attempt {
    async fn provision(&self) -> Result<Arc<dyn VmInstance>, VmError> {
        let owner = self.0.clone();
        let (reply, outcome) = tokio::sync::oneshot::channel();
        // Retained original token/operation job installed before the first await;
        // receiver abandonment never cancels launch, handoff or cached custody.
        drop(tokio::spawn(async move {
            let result = jobs::provision(&owner).await;
            let _reported = reply.send(result);
        }));
        outcome.await.map_err(|_| {
            unavailable_error(
                "original provision attempt",
                "owning provision task outcome unknown",
            )
        })?
    }
    async fn cleanup(&self) -> Result<(), VmError> {
        // Close BEFORE any await. An admitted pending launch is signaled by its
        // ORIGINAL Arc so cleanup need not wait for an uninterruptible caller.
        self.0.binding.close();
        let owner = self.0.clone();
        let (reply, outcome) = tokio::sync::oneshot::channel();
        drop(tokio::spawn(async move {
            let result = jobs::cleanup(&owner).await;
            let _reported = reply.send(result);
        }));
        outcome.await.map_err(|_| {
            unavailable_error(
                "original provision attempt",
                "owning cleanup task outcome unknown",
            )
        })?
    }
}

#[derive(Debug)]
struct OriginalError(Arc<VmError>);
impl std::fmt::Display for OriginalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self.0.as_ref(), formatter)
    }
}
impl std::error::Error for OriginalError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.0.as_ref())
    }
}
fn retained_error(error: Arc<VmError>) -> VmError {
    provider_error("original-provision-attempt", OriginalError(error))
}
