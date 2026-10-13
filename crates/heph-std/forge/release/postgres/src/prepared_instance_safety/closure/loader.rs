use capability_domain::CapabilitySlotKey;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError,
    DeploymentExecutionPreparation, DeploymentIntent, DeploymentOperation, EffectClaim,
    PreparedInstanceSafetyTarget, ResourceAction,
};
use release_domain::ContentHash;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

use super::super::{PostgresPreparedInstanceSafetyRepository, query};

#[derive(FromRow)]
struct Row {
    claim_json: serde_json::Value,
    original_actor: Uuid,
    original_idempotency: Uuid,
    canonical_input: Vec<u8>,
    preparation_hash: Vec<u8>,
}
// Exact persisted claim encoding. Historical provenance is DATA, never an identity.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ClaimWire {
    command_id: Uuid,
    deployment_id: Uuid,
    resource: String,
    identity: serde_json::Value,
    input_hash: ContentHash,
    resource_version: u64,
    generation: u64,
    action: ResourceAction,
    attempt_id: Uuid,
    actor_id: Uuid,
    request_id: Uuid,
}
pub struct Loaded {
    pub target: PreparedInstanceSafetyTarget,
    pub whole: DeploymentExecutionPreparation,
}
pub async fn load(
    adapter: &PostgresPreparedInstanceSafetyRepository,
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    intent: &DeploymentIntent,
    resource: &CapabilitySlotKey,
) -> Result<Loaded, DeploymentError> {
    let row: Row = sqlx::query_as("SELECT claim_json,original_actor,original_idempotency,canonical_input,preparation_hash FROM load_prepared_original_target118($1,$2,$3,$4,$5)")
        .bind(command.id().as_uuid()).bind(intent.id().as_uuid()).bind(resource.as_str())
        .bind(intent.project_id().as_uuid()).bind(intent.input_hash().as_bytes().as_slice())
        .fetch_one(&mut **tx).await.map_err(|error| query::storage(&error))?;
    let original = CommandIdentity::from_recorded(
        UserId::from_uuid(row.original_actor),
        RequestId::from_uuid(row.original_idempotency),
        DeploymentOperation::Install,
    )?;
    let hash = ContentHash::from_digest(
        row.preparation_hash
            .try_into()
            .map_err(|_| DeploymentError::ReconciliationRequired)?,
    );
    let whole = DeploymentExecutionPreparation::from_canonical_bytes(
        intent,
        original,
        &row.canonical_input,
        hash,
    )?;
    let prepared = whole
        .instances()
        .iter()
        .find(|child| child.resource() == resource)
        .ok_or(DeploymentError::ReconciliationRequired)?
        .clone();
    let plan = intent
        .resources()
        .get(resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    let wire: ClaimWire = serde_json::from_value(row.claim_json.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if serde_json::to_value(&wire).map_err(|_| DeploymentError::Serialization)? != row.claim_json
        || wire.command_id != original.id().as_uuid()
        || wire.deployment_id != intent.id().as_uuid()
        || wire.resource != resource.as_str()
        || wire.input_hash != plan.input_hash()
        || wire.identity
            != serde_json::to_value(plan.identity()).map_err(|_| DeploymentError::Serialization)?
        || wire.actor_id != original.actor_id().as_uuid()
        || wire.request_id.is_nil()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let claim = EffectClaim {
        command: original,
        deployment_id: intent.id(),
        resource: resource.clone(),
        identity: plan.identity(),
        input_hash: wire.input_hash,
        resource_version: wire.resource_version,
        generation: wire.generation,
        action: wire.action,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(wire.attempt_id)?,
            actor_id: original.actor_id(),
            request_id: RequestId::from_uuid(wire.request_id),
        },
    };
    let target = PreparedInstanceSafetyTarget::new(
        claim,
        prepared,
        adapter.namespace.clone(),
        adapter.host.clone(),
    )?;
    adapter.configured(&target)?;
    Ok(Loaded { target, whole })
}
