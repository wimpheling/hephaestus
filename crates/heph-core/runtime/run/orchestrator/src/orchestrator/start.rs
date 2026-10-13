use run_domain::{Run, RunKind, RunState, StartRun};

use super::{OrchestratorError, RunOrchestrator, complete, prepare, provision};

impl RunOrchestrator {
    /// Executes an idempotent start command through complete cleanup.
    ///
    /// # Errors
    ///
    /// Returns an error when durable state, volume, or VM operations fail.
    /// Cleanup failures deliberately retain the volume lease for recovery.
    pub async fn start_run(&self, command: &StartRun) -> Result<Run, OrchestratorError> {
        if command.kind == RunKind::Invocation {
            self.require_invocation_configuration(command)?;
        }
        if self.plural_volumes && self.canonical_cleanup.is_none() {
            return Err(volume_trait::VolumeError::InvalidState(
                "complete-set preparation requires canonical cleanup",
            )
            .into());
        }
        if self.legacy_scope.is_some() {
            if self.canonical_cleanup.is_some() || self.plural_volumes {
                return Err(crate::RepositoryError::InvalidData(
                    "strict Legacy and canonical modes are exclusive",
                )
                .into());
            }
            self.legacy_owner()?;
            self.operation_guards.check_io(command.run_id)?;
        }
        let claim = if self.canonical_cleanup.is_some() || self.legacy_scope.is_some() {
            self.operation_guards.check_io(command.run_id)?;
            Some(self.operation_guards.claim_start(command.run_id)?)
        } else {
            None
        };
        // Keep the public future small while the internal complete-set pipeline
        // carries bounded attachment and monitor evidence. Cancellation still
        // drops that exact pipeline before releasing the live-start claim.
        let result = Box::pin(self.start_claimed_run(command)).await;
        // The admission token spans the whole future, including cleanup; the
        // independent physical operation mutex never spans guest execution.
        drop(claim);
        result
    }

    async fn start_claimed_run(&self, command: &StartRun) -> Result<Run, OrchestratorError> {
        let created = if self.canonical_cleanup.is_some() {
            self.create_planned_run(command).await?
        } else if self.legacy_scope.is_some() {
            self.create_legacy_run(command).await?
        } else {
            self.repository.create_run(command).await?
        };
        if !created.created {
            match created.run.state {
                RunState::Queued | RunState::LeasingVolume => {}
                RunState::CleanedUp => {
                    if self.legacy_scope.is_some() {
                        return self.legacy_terminal(command.run_id).await;
                    }
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
        let result = async {
            self.bind_planned_vm(&created.run).await?;
            self.execute_claimed_run(command, &created.run).await
        }
        .await;
        if self.legacy_scope.is_some() {
            return self
                .finish_legacy_start_result(command.run_id, result)
                .await;
        }
        if self.canonical_cleanup.is_none() {
            return result;
        }
        match result {
            Err(
                error @ (OrchestratorError::CleanupIncomplete { .. }
                | OrchestratorError::RunInProgress(_)),
            ) => Err(error),
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
        if self.plural_volumes {
            return self.execute_monitored_run(command.run_id, prepared).await;
        }
        self.execute_prepared_run(command.run_id, prepared, None)
            .await
    }

    pub(super) async fn execute_prepared_run(
        &self,
        run_id: runtime_types::RunId,
        prepared: prepare::PreparedStart,
        guest_cleanup: Option<&super::plural_monitor::GuestCleanupPhase>,
    ) -> Result<Run, OrchestratorError> {
        let started = match provision::provision_and_start(self, run_id, prepared).await? {
            provision::ProvisionResult::Finished(run) => return Ok(*run),
            provision::ProvisionResult::Started(started) => *started,
        };
        complete::complete_started_run(self, run_id, started, guest_cleanup).await
    }
}
