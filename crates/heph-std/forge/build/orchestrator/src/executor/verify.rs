use super::{
    BuildExecutionError, BuildExecutor, BuildRequestId, VmId, artifact_manifest, cleanup_workspace,
    release_inputs, seal_output, validate_declared_outputs,
};

impl BuildExecutor {
    /// Re-executes immutable build inputs and compares the produced manifest
    /// with the original draft release without mutating that release.
    ///
    /// # Errors
    ///
    /// Returns a durable repository, VM, artifact, or verification failure.
    pub async fn verify(
        &self,
        build_request_id: BuildRequestId,
    ) -> Result<(), BuildExecutionError> {
        super::owner::verify(self.clone(), build_request_id).await
    }

    pub(super) async fn verify_owned(
        &self,
        build_request_id: BuildRequestId,
        node: &std::sync::Arc<super::owner::Node>,
    ) -> Result<(), BuildExecutionError> {
        node.check_open()?;
        self.ensure_image_available(build_request_id).await?;
        node.check_open()?;
        let claimed = self.repository.claim_verification(build_request_id).await?;
        let workspace = match self.materialize(&claimed, node).await {
            Ok(workspace) => workspace,
            Err(error) => {
                self.repository
                    .fail_verification(build_request_id, "source_materialization")
                    .await?;
                return Err(error);
            }
        };
        self.build_guest(&claimed, &workspace, node, true).await?;
        node.original_workspace()?.recheck()?;
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
        node.remove_workspace()?;
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
