use async_trait::async_trait;
use identity_domain::AuthenticatedIdentity;
use std::sync::{Arc, Mutex};
use volume_trait::{
    BeginOwnedProvisioning, OwnedBackingObservation, OwnedBackingPhase, OwnedProvisioningClaim,
    OwnedProvisioningContext, VolumeError, VolumeOwnedProvisioningRepository,
};

pub struct State {
    pub context: OwnedProvisioningContext,
    pub authorized: bool,
    pub begins: usize,
    pub reject_begin: Option<usize>,
    pub fail_ready_once: bool,
    pub revoke_on_format_record: bool,
}
pub struct Metadata {
    pub state: Arc<Mutex<State>>,
    pub worker: bool,
}
#[async_trait]
impl VolumeOwnedProvisioningRepository for Metadata {
    async fn begin_owned_provisioning(
        &self,
        identity: &AuthenticatedIdentity,
        request: &BeginOwnedProvisioning,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let mut state = self.state.lock().expect("state");
        state.begins += 1;
        if self.worker
            || !state.authorized
            || state.reject_begin == Some(state.begins)
            || identity.user_id.as_uuid()
                != state
                    .context
                    .claim
                    .purpose
                    .receipt()
                    .intent
                    .creation()
                    .fields()
                    .actor_id
        {
            return Err(VolumeError::PermissionDenied);
        }
        if request.purpose != state.context.claim.purpose
            || request.operation_id != state.context.claim.operation_id
            || request.expected_generation != 0
        {
            return Err(VolumeError::IntentConflict);
        }
        Ok(state.context.clone())
    }
    async fn owned_reconciliation_context(
        &self,
        claim: &OwnedProvisioningClaim,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let state = self.state.lock().expect("state");
        if !self.worker {
            return Err(VolumeError::PermissionDenied);
        }
        if *claim != state.context.claim {
            return Err(VolumeError::IntentConflict);
        }
        Ok(state.context.clone())
    }
    async fn record_owned_observation(
        &self,
        claim: &OwnedProvisioningClaim,
        observation: &OwnedBackingObservation,
    ) -> Result<OwnedProvisioningContext, VolumeError> {
        let mut state = self.state.lock().expect("state");
        if !self.worker {
            return Err(VolumeError::PermissionDenied);
        }
        if *claim != state.context.claim
            || observation.fields().purpose_hash != claim.purpose.hash()
            || observation.fields().generation != claim.generation
        {
            return Err(VolumeError::IntentConflict);
        }
        if observation.fields().phase == OwnedBackingPhase::Ready && state.fail_ready_once {
            state.fail_ready_once = false;
            return Err(VolumeError::InvalidState(
                "simulated final metadata commit failure",
            ));
        }
        state.context.observation = Some(observation.clone());
        if state.revoke_on_format_record
            && observation.fields().phase == OwnedBackingPhase::FormatIntent
        {
            state.authorized = false;
        }
        Ok(state.context.clone())
    }
}
