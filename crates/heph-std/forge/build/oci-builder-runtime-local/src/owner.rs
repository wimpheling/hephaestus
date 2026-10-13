//! Original OCI operation custody; not whole OCI/prior-process shutdown proof.
mod cleanup;
mod errors;
mod guest;
mod jobs;
mod mkfs;
mod models;
mod node;
mod pin;
mod preparation;
mod registry;
mod settlement;
mod task;
#[cfg(all(test, target_os = "linux"))]
mod tests;
pub use models::{OciCleanupHeld, OciCleanupPhase, OciCleanupStatus};
use node::Node;
use oci_builder_worker::OciWorkerError;
use registry::Registry;
use std::sync::{Mutex, MutexGuard};
pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Concrete original operation roots/registry, private to this adapter.
pub struct Owner {
    roots: pin::Roots,
    registry: Registry,
    errors: Mutex<Vec<std::sync::Arc<dyn std::error::Error + Send + Sync>>>,
}
impl Owner {
    pub(super) fn initialize(
        candidate: &std::path::Path,
        scratch: &std::path::Path,
        verification: &std::path::Path,
        program: &std::path::Path,
    ) -> Result<Self, OciWorkerError> {
        Ok(Self {
            roots: pin::Roots::open(candidate, scratch, verification, program)?,
            registry: Registry::default(),
            errors: Mutex::new(Vec::new()),
        })
    }
    pub(super) fn close(&self) {
        self.registry.close();
    }
}
pub async fn execute(
    operation: crate::VmOciOperation,
    request: oci_builder_worker::IsolatedOciBuild,
) -> Result<crate::VerifiedVmOciOutput, OciWorkerError> {
    jobs::execute(operation, request).await
}
