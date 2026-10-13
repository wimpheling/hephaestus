//! Selected production operations; controller requalification precedes IO.

use super::{ManagedOrdinaryAdmissionPolicy, ManagedRunCommandExecutor, held, routing::Selection};
use async_trait::async_trait;
use heph_run::{CancelRun, StartRun};
use run_orchestrator::{
    OrchestratorError, RunCleanupWakeProgress, RunCleanupWakeRequest, RunCommandExecutor,
};
use std::sync::Arc;

#[async_trait]
impl RunCommandExecutor for ManagedRunCommandExecutor {
    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        match self.select_start(command).await? {
            Selection::Invocation => {
                let owner = Arc::clone(&self.invocation);
                let result = owner.orchestrator().start_run(command).await.map(|_| ());
                drop(owner);
                result
            }
            Selection::LegacyOriginal => Arc::clone(&self.legacy)
                .start_run(command)
                .await
                .map(|_| ()),
            Selection::LegacyProspective => match self.ordinary_policy {
                ManagedOrdinaryAdmissionPolicy::LegacyCompatibility => {
                    // The reader owns no lock or admission. This actual core path
                    // persists strict profile/placement under the consumer lock
                    // before resources/provider IO, and rejects conflicting birth.
                    Arc::clone(&self.legacy)
                        .start_run(command)
                        .await
                        .map(|_| ())
                }
            },
        }
    }

    async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        match self.select_original(command.run_id).await? {
            Selection::Invocation => Arc::clone(&self.invocation).cancel_run(command).await,
            Selection::LegacyOriginal => Arc::clone(&self.legacy).cancel_run(command).await,
            Selection::LegacyProspective => Err(held(
                "managed Run Cancel is held: unborn request has no durable original profile",
            )),
        }
    }

    async fn wake_cleanup(
        self: Arc<Self>,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        let result = match self.select_original(request.run_id()).await? {
            Selection::Invocation => Arc::clone(&self.invocation).wake_cleanup(request).await,
            Selection::LegacyOriginal => Err(held(
                "managed Run Wake is held: PostgreSQL strict Legacy Wake is unsupported",
            )),
            Selection::LegacyProspective => Err(held(
                "managed Run Wake is held: unborn request has no durable original profile",
            )),
        };
        // Retain BOTH actual controller owners throughout the awaited operation.
        // Existing Invocation Wake independently requalifies original removal,
        // cancellation, closure and owner; its progress is no Drain/stop proof.
        drop(self);
        result
    }
}
