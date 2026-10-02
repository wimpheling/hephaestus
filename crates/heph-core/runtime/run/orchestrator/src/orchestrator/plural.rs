use std::time::Duration;

use run_domain::Run;
use volume_domain::RunVolumeSelections;
use volume_trait::VolumeError;

use super::{
    OrchestratorError, RunOrchestrator,
    plural_evidence::{PreparedRunVolumes, validate_identity},
};

const LIVE_CHECK_LIMIT: Duration = Duration::from_secs(2);

impl RunOrchestrator {
    pub(super) async fn acquire_run_volumes(
        &self,
        run: &Run,
    ) -> Result<PreparedRunVolumes, OrchestratorError> {
        let cleanup = self
            .canonical_cleanup
            .as_ref()
            .ok_or(VolumeError::InvalidState(
                "complete-set preparation requires canonical cleanup",
            ))?;
        let selections = cleanup.volumes.load_run_selections(run.id).await?;
        validate_identity(run, &selections)?;
        self.check_volume_authority(run, &selections).await?;
        let attachments = cleanup.volumes.acquire_run(&selections).await?;
        let scope = self.provider.owner_scope()?;
        Ok(PreparedRunVolumes::new(
            run,
            selections,
            attachments,
            scope.host_id(),
        )?)
    }

    /// One deadline covers caller/source, secrets, runtime capabilities and mounts.
    pub(super) async fn check_volume_authority(
        &self,
        run: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<(), OrchestratorError> {
        tokio::time::timeout(LIVE_CHECK_LIMIT, async {
            self.check_volume_sources(run, selections).await?;
            self.run_volume_store()?.preflight_run(selections).await?;
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| deadline())?
    }

    pub(super) async fn check_draining_authority(
        &self,
        run: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<(), OrchestratorError> {
        // Only the owned normal drain phase uses this: no further guest Start,
        // no lease reuse, and all canonical fences remain held. Closure is
        // intentional; caller/source/runtime/secret checks continue independently.
        tokio::time::timeout(LIVE_CHECK_LIMIT, self.check_volume_sources(run, selections))
            .await
            .map_err(|_| deadline())?
    }

    async fn check_volume_sources(
        &self,
        run: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<(), OrchestratorError> {
        validate_identity(run, selections)?;
        let current = self.repository.get(run.id).await?;
        validate_identity(&current, selections)?;
        if current.cancel_requested_at.is_some() {
            return Err(
                VolumeError::InvalidState("run cancellation closes volume execution").into(),
            );
        }
        self.launch_authorizer
            .authorize_with_volumes(&current)
            .await
            .map_err(|error| super::authority::RunAuthorityError::redacted(error.to_string()))?;
        self.secrets.reauthorize(&current).await?;
        self.authority.reauthorize(&current).await?;
        Ok(())
    }

    pub(super) async fn refresh_run_volumes(
        &self,
        run: &Run,
        volumes: &mut PreparedRunVolumes,
        attached: bool,
    ) -> Result<(), OrchestratorError> {
        tokio::time::timeout(LIVE_CHECK_LIMIT, async {
            self.check_volume_sources(run, &volumes.selections).await?;
            let store = self.run_volume_store()?;
            let leases = if attached {
                store.mark_run_attached(&volumes.selections).await?
            } else {
                store.heartbeat_run(&volumes.selections).await?
            };
            let scope = self.provider.owner_scope()?;
            volumes.refreshed(leases, scope.host_id())?;
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| deadline())?
    }

    fn run_volume_store(&self) -> Result<&dyn volume_trait::RunVolumeStore, VolumeError> {
        self.canonical_cleanup
            .as_ref()
            .map(|cleanup| cleanup.volumes.as_ref())
            .ok_or(VolumeError::InvalidState(
                "canonical volume store is absent",
            ))
    }
}

fn deadline() -> OrchestratorError {
    VolumeError::InvalidState("combined live volume authorization deadline elapsed").into()
}
