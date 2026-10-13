//! Admitted commands own the complete original Core future, not only an Arc.

use super::{
    ManagedOrdinaryAdmissionPolicy, ManagedRunCommandExecutor, held, intake, ownership,
    routing::Selection,
};
use async_trait::async_trait;
use heph_run::{CancelRun, StartRun};
use run_orchestrator::{
    OrchestratorError, RunCleanupWakeProgress, RunCleanupWakeRequest, RunCommandExecutor,
};
use std::sync::Arc;

#[async_trait]
impl RunCommandExecutor for ManagedRunCommandExecutor {
    async fn start_run(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        let owner = self.clone();
        let command = command.clone();
        // SAME synchronous intake lock registers before profile inspection or IO.
        let receiver = ownership::spawn(
            &self.operations,
            intake::Request::Start(command.clone()),
            async move { owner.start_selected(&command).await },
        )?;
        ownership::receive(receiver).await
    }

    async fn cancel_run(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        let owner = self.clone();
        let command = command.clone();
        let receiver = ownership::spawn(
            &self.operations,
            intake::Request::Cancel(command.clone()),
            async move { owner.cancel_selected(&command).await },
        )?;
        ownership::receive(receiver).await
    }

    async fn wake_cleanup(
        self: Arc<Self>,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        let owner = self.as_ref().clone();
        let request = request.clone();
        let receiver = ownership::spawn(
            &self.operations,
            intake::Request::Wake(request.clone()),
            async move { owner.wake_selected(&request).await },
        )?;
        ownership::receive(receiver).await
    }
}

impl ManagedRunCommandExecutor {
    async fn start_selected(&self, command: &StartRun) -> Result<(), OrchestratorError> {
        match self.select_start(command).await? {
            Selection::Invocation => self
                .invocation
                .orchestrator()
                .start_run(command)
                .await
                .map(|_| ()),
            Selection::LegacyOriginal => Arc::clone(&self.legacy)
                .start_run(command)
                .await
                .map(|_| ()),
            Selection::LegacyProspective => match self.ordinary_policy {
                ManagedOrdinaryAdmissionPolicy::LegacyCompatibility => Arc::clone(&self.legacy)
                    .start_run(command)
                    .await
                    .map(|_| ()),
            },
        }
    }

    async fn cancel_selected(&self, command: &CancelRun) -> Result<bool, OrchestratorError> {
        match self.select_original(command.run_id).await? {
            Selection::Invocation => Arc::clone(&self.invocation).cancel_run(command).await,
            Selection::LegacyOriginal => Arc::clone(&self.legacy).cancel_run(command).await,
            Selection::LegacyProspective => Err(held(
                "managed Run Cancel is held: unborn request has no durable original profile",
            )),
        }
    }

    async fn wake_selected(
        &self,
        request: &RunCleanupWakeRequest,
    ) -> Result<RunCleanupWakeProgress, OrchestratorError> {
        match self.select_original(request.run_id()).await? {
            Selection::Invocation => Arc::clone(&self.invocation).wake_cleanup(request).await,
            Selection::LegacyOriginal => Err(held(
                "managed Run Wake is held: PostgreSQL strict Legacy Wake is unsupported",
            )),
            Selection::LegacyProspective => Err(held(
                "managed Run Wake is held: unborn request has no durable original profile",
            )),
        }
    }
}
