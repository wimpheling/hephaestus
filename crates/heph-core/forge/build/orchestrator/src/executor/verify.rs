use super::{
    BuildExecutionError, BuildExecutor, BuildRequestId, StopMode, VmId, artifact_manifest,
    cleanup_workspace, collect_execution, prepare_workspace, release_inputs, seal_output,
    validate_declared_outputs,
};

impl BuildExecutor {
    /// Re-executes immutable build inputs and compares the produced manifest
    /// with the original draft release without mutating that release.
    ///
    /// # Errors
    ///
    /// Returns a durable repository, VM, artifact, or verification failure.
    #[allow(clippy::too_many_lines)]
    pub async fn verify(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<(), BuildExecutionError> {
        self.ensure_image_available(build_request_id).await?;
        let claimed = self.repository.claim_verification(build_request_id).await?;
        let workspace = self.active_path(build_request_id);
        let materializer = self.config.clone();
        let input = claimed.input.clone();
        let prepared = tokio::task::spawn_blocking(move || {
            prepare_workspace(&materializer, &input, &workspace)
        })
        .await
        .map_err(|_| BuildExecutionError::WorkerJoin)?;
        let workspace = match prepared {
            Ok(workspace) => workspace,
            Err(error) => {
                self.repository
                    .fail_verification(build_request_id, "source_materialization")
                    .await?;
                return Err(error);
            }
        };
        let spec = match self.vm_spec(&claimed, &workspace) {
            Ok(spec) => spec,
            Err(error) => {
                self.repository
                    .fail_verification(build_request_id, "invalid_build_contract")
                    .await?;
                cleanup_workspace(&workspace.root)?;
                return Err(error);
            }
        };
        let Ok(instance) = self.provider.provision(spec).await else {
            self.repository
                .fail_verification(build_request_id, "vm_provision")
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        };
        let mut events = instance.subscribe_events();
        if instance.start().await.is_err() {
            drop(instance.destroy().await);
            self.repository
                .fail_verification(build_request_id, "vm_start")
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::Vm);
        }
        let (exit, _logs, _metrics, timed_out) =
            collect_execution(&instance, &mut events, self.config.timeout).await;
        if timed_out {
            drop(instance.stop(StopMode::Force).await);
        }
        if instance.destroy().await.is_err() {
            self.repository
                .fail_verification(build_request_id, "vm_destroy")
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::VmCleanup);
        }
        let exit = exit.ok_or(BuildExecutionError::Vm)?;
        if timed_out || exit.code != Some(0) || exit.signal.is_some() {
            self.repository
                .fail_verification(build_request_id, "guest_failed")
                .await?;
            cleanup_workspace(&workspace.root)?;
            return Err(BuildExecutionError::GuestFailed);
        }
        seal_output(&workspace.output)?;
        let imported = self
            .artifacts
            .import_for(build_request_id.as_uuid(), &workspace.output)?;
        validate_declared_outputs(&claimed.input.build, &imported)?;
        let artifacts = release_inputs(build_request_id, &claimed.input.build, &imported)?;
        let matches = self
            .repository
            .complete_verification(build_request_id, &artifact_manifest(&artifacts))
            .await?;
        cleanup_workspace(&workspace.root)?;
        if matches {
            Ok(())
        } else {
            Err(BuildExecutionError::VerificationMismatch)
        }
    }

    /// Reaps build VMs abandoned before the one-way sealed-output boundary.
    ///
    /// The durable release and execution identities are retained. Once
    /// provider cleanup is confirmed, redelivery may safely retry the exact
    /// build without creating another identity.
    ///
    /// # Errors
    ///
    /// Returns an error unless every selected orphan is confirmed destroyed
    /// and its private transient workspace is removed.
    pub async fn recover_after_restart(&self) -> Result<usize, BuildExecutionError> {
        let rows = self.repository.recoverable().await?;
        for row in &rows {
            self.provider
                .cleanup_orphan(&VmId(row.vm_id.clone()))
                .await
                .map_err(|_| BuildExecutionError::VmCleanup)?;
            cleanup_workspace(&self.active_path(row.id))?;
            self.repository.reset_after_cleanup(row.id).await?;
        }
        let finalizing = self.repository.finalizing().await?;
        for id in &finalizing {
            self.execute(*id).await?;
        }
        Ok(rows.len() + finalizing.len())
    }
}
