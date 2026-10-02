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
                    if self.canonical_cleanup.is_some() {
                        return self.canonical_cleanup(command.run_id, None).await;
                    }
                    self.completion.after_cleanup(&created.run).await?;
                    return Ok(created.run);
                }
                _ => return Err(OrchestratorError::RunInProgress(command.run_id)),
            }
        }
        // New runs persist the actual provider owner and planned identity
        // before any preparation, authority check or acquisition effect.
        self.bind_planned_vm(&created.run, created.created).await?;
        let result = self.execute_claimed_run(command, &created.run).await;
        if self.canonical_cleanup.is_none() {
            return result;
        }
        match result {
            Err(error @ OrchestratorError::CleanupIncomplete { .. }) => Err(error),
            Err(error) => {
                if let Ok(current) = self.repository.get(command.run_id).await
                    && current.state == RunState::CleanedUp
                {
                    return Err(error);
                }
                let failure = error.to_string();
                if let Err(cleanup) = self.fail_claimed_run(command.run_id, &failure).await {
                    return Err(OrchestratorError::CleanupIncomplete {
                        failure,
                        cleanup: Box::new(cleanup),
                    });
                }
                // Completing cleanup does not erase an unexpected primary error.
                Err(error)
            }
            Ok(run) => Ok(run),
        }
    }

    async fn execute_claimed_run(
        &self,
        command: &StartRun,
        initial: &Run,
    ) -> Result<Run, OrchestratorError> {
        if initial.state == RunState::Queued {
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
