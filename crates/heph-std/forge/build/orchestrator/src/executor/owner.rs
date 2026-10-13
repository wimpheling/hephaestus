//! Live original build custody. No prior-process/root absence certificate.
mod cleanup;
mod context;
mod git_cleanup;
mod jobs;
mod models;
mod node;
mod preparation;
mod registry;
mod settlement;
mod task;
use super::{BuildExecutionError, BuildExecutionResult, BuildExecutor, BuildRequestId};
use models::Mode;
pub use models::{BuildCleanupError, BuildCleanupPhase, BuildCleanupStatus};
pub(super) use node::Node;
pub(super) use registry::Registry;
use std::sync::{Mutex, MutexGuard};

pub(super) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
#[cfg(all(test, target_os = "linux"))]
mod tests;

pub(super) async fn execute(
    executor: BuildExecutor,
    id: BuildRequestId,
) -> Result<BuildExecutionResult, BuildExecutionError> {
    jobs::execute(executor, id, Mode::Execute).await
}
pub(super) async fn retry(
    executor: BuildExecutor,
    id: BuildRequestId,
) -> Result<BuildExecutionResult, BuildExecutionError> {
    jobs::execute(executor, id, Mode::Retry).await
}
pub(super) async fn verify(
    executor: BuildExecutor,
    id: BuildRequestId,
) -> Result<(), BuildExecutionError> {
    jobs::verify(executor, id).await
}
impl BuildExecutor {
    /// Continues only this live owner's original sealed/imported output even
    /// after intake closes; cannot claim/reset/provision a new execution.
    /// # Errors
    /// Missing/changed original finalization context or publication is held.
    pub async fn resume_finalization_owned(
        &self,
        id: BuildRequestId,
    ) -> Result<BuildExecutionResult, BuildExecutionError> {
        jobs::execute(self.clone(), id, Mode::Finalize).await
    }
}
