use run_domain::{Run, RunState, StartRun};

use super::{OrchestratorError, RunOrchestrator, complete, prepare, provision};

impl RunOrchestrator {
    /// Executes an idempotent start command through complete cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when durable state, volume, or VM operations fail.
    /// Cleanup failures deliberately retain the volume lease for recovery.
    pub async fn start_run(&self, command: &StartRun) -> Result<Run, OrchestratorError> {
        let created = self.repository.create_run(command).await?;
        if !created.created {
            match created.run.state {
                RunState::Queued | RunState::LeasingVolume => {}
                RunState::CleanedUp => {
                    self.completion.after_cleanup(&created.run).await?;
                    return Ok(created.run);
                }
                _ => return Err(OrchestratorError::RunInProgress(command.run_id)),
            }
        }
        if created.run.state == RunState::Queued {
            self.repository
                .transition(command.run_id, RunState::LeasingVolume, None, None)
                .await?;
        }
        let prepared = match prepare::prepare_start(self, command).await? {
            prepare::PrepareStart::Finished(run) => return Ok(*run),
            prepare::PrepareStart::Continue(prepared) => *prepared,
        };
        let started = match provision::provision_and_start(self, command.run_id, prepared).await? {
            provision::ProvisionResult::Finished(run) => return Ok(*run),
            provision::ProvisionResult::Started(started) => *started,
        };
        complete::complete_started_run(self, command.run_id, started).await
    }
}
