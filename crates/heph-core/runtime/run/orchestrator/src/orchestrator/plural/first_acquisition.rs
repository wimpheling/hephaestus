//! Initial Invocation eligibility only; later checks retain full secret proof.
use run_domain::{Run, RunKind, RunState};
use volume_domain::RunVolumeSelections;
use volume_trait::VolumeError;

use super::{LIVE_CHECK_LIMIT, OrchestratorError, RunOrchestrator, deadline, validate_identity};

impl RunOrchestrator {
    pub(super) async fn check_invocation_first_acquisition(
        &self,
        run: &Run,
        selections: &RunVolumeSelections,
    ) -> Result<(), OrchestratorError> {
        tokio::time::timeout(LIVE_CHECK_LIMIT, async {
            let expected = self
                .invocation_scope
                .as_ref()
                .ok_or(OrchestratorError::InvocationUnsupported)?;
            if !self.plural_volumes
                || self.canonical_cleanup.is_none()
                || self.legacy_scope.is_some()
                || &self
                    .provider
                    .owner_scope()
                    .map_err(|_| OrchestratorError::InvocationUnsupported)?
                    != expected
            {
                return Err(OrchestratorError::InvocationUnsupported);
            }
            validate_identity(run, selections)?;
            let current = self.repository.get(run.id).await?;
            validate_identity(&current, selections)?;
            if &current != run
                || current.kind != RunKind::Invocation
                || current.state != RunState::LeasingVolume
                || current.attachment_id.is_some()
                || current.volume_id.is_some()
                || current.lease_id.is_some()
                || current.lease_fencing_token.is_some()
                || current.cancel_requested_at.is_some()
                || current.outcome.is_some()
                || current.exit.is_some()
                || current.failure.is_some()
                || current.vm_id.as_deref() != Some(current.id.to_string().as_str())
            {
                return Err(VolumeError::InvalidState(
                    "Invocation first acquisition differs from exact fresh persisted run",
                )
                .into());
            }
            // Run has no profile/history projection. Both configured ports must
            // positively verify the protected producer and first-acquisition
            // facade; local kind/config checks cannot supply that authority.
            self.launch_authorizer
                .authorize_with_volumes(&current)
                .await
                .map_err(|error| {
                    super::super::authority::RunAuthorityError::redacted(error.to_string())
                })?;
            self.authority.reauthorize(&current).await?;
            self.run_volume_store()?.preflight_run(selections).await?;
            // Only this initial check defers secrets. bind_start's full check on
            // the transition-returned Provisioning Run precedes runtime/VM IO,
            // including zero slots; every later check stays unchanged.
            Ok::<(), OrchestratorError>(())
        })
        .await
        .map_err(|_| deadline())?
    }
}
