use super::{RecipeRemovalError, Request};
use capability_domain::CapabilitySlotKey;
use recipe_application::{
    CommandIdentity, CommandReceipt, DeploymentIntent, PreparedOwnedVolumeCreation,
    RetainOwnedReady,
};
use std::{collections::BTreeMap, sync::Arc};
use volume_trait::OwnedProvisioningClaim;

pub(super) struct Volume {
    pub request: RetainOwnedReady,
    pub preparation: PreparedOwnedVolumeCreation,
    pub claim: OwnedProvisioningClaim,
}
pub(super) struct Final {
    pub command: CommandIdentity,
    pub before: u64,
    pub receipt: Option<CommandReceipt>,
}
pub(super) struct Original {
    pub requests: Vec<Request>,
    pub intent: Option<DeploymentIntent>,
    pub instance_cas: BTreeMap<(CapabilitySlotKey, uuid::Uuid), u64>,
    pub volumes: BTreeMap<CapabilitySlotKey, Vec<Volume>>,
    pub finalizations: Vec<Final>,
    pub errors: Vec<Arc<RecipeRemovalError>>,
}
impl Original {
    pub fn new(request: Request) -> Self {
        Self {
            requests: vec![request],
            intent: None,
            instance_cas: BTreeMap::new(),
            volumes: BTreeMap::new(),
            finalizations: Vec::new(),
            errors: Vec::new(),
        }
    }
    pub fn request(&mut self, current: &Request) -> Result<(), RecipeRemovalError> {
        if current.remove.deployment_id != self.requests[0].remove.deployment_id {
            return Err(recipe_application::DeploymentError::InputConflict.into());
        }
        if let Some(prior) = self
            .requests
            .iter()
            .find(|prior| prior.remove.command == current.remove.command)
        {
            if prior.remove != current.remove || prior.mode != current.mode {
                return Err(recipe_application::DeploymentError::InputConflict.into());
            }
        } else {
            self.requests.push(current.clone());
        }
        Ok(())
    }
    pub fn intent(&mut self, current: &DeploymentIntent) -> Result<(), RecipeRemovalError> {
        if self.intent.as_ref().is_some_and(|prior| prior != current) {
            return Err(recipe_application::DeploymentError::IntentMismatch.into());
        }
        if self.intent.is_none() {
            self.intent = Some(current.clone());
        }
        Ok(())
    }
}
