use recipe_application::{DeploymentError, EffectClaim, PlannedResourceIdentity};
use release_domain::ContentHash;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimWire {
    pub command_id: Uuid,
    pub deployment_id: Uuid,
    pub resource: String,
    pub identity: serde_json::Value,
    pub input_hash: ContentHash,
    pub resource_version: u64,
    pub generation: u64,
    pub action: recipe_application::ResourceAction,
    pub attempt_id: Uuid,
    pub actor_id: Uuid,
    pub request_id: Uuid,
}

impl ClaimWire {
    pub fn new(claim: &EffectClaim) -> Result<Self, DeploymentError> {
        Ok(Self {
            command_id: claim.command.id().as_uuid(),
            deployment_id: claim.deployment_id.as_uuid(),
            resource: claim.resource.as_str().into(),
            identity: identity(claim.identity)?,
            input_hash: claim.input_hash,
            resource_version: claim.resource_version,
            generation: claim.generation,
            action: claim.action,
            attempt_id: claim.provenance.attempt_id.as_uuid(),
            actor_id: claim.provenance.actor_id.as_uuid(),
            request_id: claim.provenance.request_id.as_uuid(),
        })
    }
}

pub fn identity(value: PlannedResourceIdentity) -> Result<serde_json::Value, DeploymentError> {
    serde_json::to_value(value).map_err(|_| DeploymentError::Serialization)
}

#[derive(FromRow)]
pub struct AttemptRow {
    pub claim_json: sqlx::types::Json<ClaimWire>,
    pub before_state: sqlx::types::Json<crate::effect_state::State>,
}

#[derive(FromRow)]
pub struct TransitionRow {
    pub fingerprint: Vec<u8>,
    pub after_state: Option<sqlx::types::Json<crate::effect_state::State>>,
}
