use super::super::PostgresOwnedReadyRetentionRepository;
use crate::{
    effect_context::Context, execution_rows::ClaimWire, hydration, owned_creation_preparation,
    repository_error,
};
use capability_domain::CapabilitySlotKey;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentId,
    DeploymentOperation, EffectClaim, PlannedResourceIdentity, ResourceAction,
    RetainedExternalVerificationPlan,
};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Origin {
    closure_id: Uuid,
    retention_operation: Uuid,
    deployment_id: Uuid,
    resource_name: String,
    claim_json: sqlx::types::Json<ClaimWire>,
    command_actor: Uuid,
    idempotency_id: Uuid,
}

pub(super) async fn load(
    adapter: &PostgresOwnedReadyRetentionRepository,
    context: &mut Context<'_>,
    claim: &EffectClaim,
) -> Result<RetainedExternalVerificationPlan, DeploymentError> {
    let PlannedResourceIdentity::Volume {
        id,
        filesystem_uuid,
    } = claim.identity
    else {
        return Err(DeploymentError::InvalidAction);
    };
    let project = context.snapshot.intent.project_id().as_uuid();
    let row = find(
        &mut context.tx,
        id.as_uuid(),
        project,
        &adapter.namespace,
        &adapter.host,
    )
    .await?;
    let old = hydration::load(&mut context.tx, DeploymentId::from_uuid(row.deployment_id)?).await?;
    let name = CapabilitySlotKey::parse(row.resource_name.clone())
        .map_err(|_| DeploymentError::InputConflict)?;
    let original = restore(&row, &old, &name)?;
    let prepared = owned_creation_preparation::storage::load(&mut context.tx, &original)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?;
    if prepared != owned_creation_preparation::mapping::derive(&old.intent, &original)?
        || prepared.volume_id != id
        || prepared.project_id != project
        || filesystem_uuid.is_some_and(|expected| expected != prepared.filesystem_uuid)
    {
        return Err(DeploymentError::InputConflict);
    }
    let target = context
        .snapshot
        .intent
        .resources()
        .get(&claim.resource)
        .ok_or(DeploymentError::Unavailable)?;
    let recipe_domain::ResolvedResource::Volume(volume) = target.intent() else {
        return Err(DeploymentError::InvalidAction);
    };
    if target.identity() != claim.identity
        || target.input_hash() != claim.input_hash
        || volume.external_id != Some(id)
        || volume.capacity_bytes != prepared.capacity_bytes
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(RetainedExternalVerificationPlan {
        claim: claim.clone(),
        preparation: prepared,
        closure_id: row.closure_id,
        retention_operation_id: DeploymentAttemptId::from_uuid(row.retention_operation)?,
        vm_provider_namespace: adapter.namespace.clone(),
        vm_host_id: adapter.host.clone(),
        completed: None,
    })
}

async fn find(
    tx: &mut Transaction<'_, Postgres>,
    volume: Uuid,
    project: Uuid,
    namespace: &str,
    host: &str,
) -> Result<Origin, DeploymentError> {
    let rows: Vec<Origin> = sqlx::query_as(
        "SELECT closure.id AS closure_id,retention.operation_id AS retention_operation,
          closure.deployment_id,closure.resource_name,original.claim_json,
          command.actor_id AS command_actor,command.idempotency_id
         FROM volume_owned_ready_retention_closures closure
         JOIN agent_instance_state_volumes volume ON volume.id=closure.volume_id
          AND volume.project_id=closure.project_id AND volume.ready_retention_closure_id=closure.id
         JOIN recipe_owned_ready_retentions retained ON retained.closure_id=closure.id
         JOIN volume_owned_ready_retention_facts fact ON fact.id=retained.fact_id AND fact.closure_id=closure.id
         JOIN volume_owned_backing_observations ready ON ready.id=closure.ready_observation_id
          AND ready.volume_id=volume.id AND ready.operation_id=closure.provisioning_operation_id
          AND ready.generation=1 AND ready.phase='ready'
         JOIN recipe_execution_transitions retention ON retention.id=retained.transition_id
          AND retention.kind='creation_retained_ready' AND retention.ready_retention_fact_id=fact.id
          AND retention.attempt_id=closure.attempt_id AND retention.project_id=closure.project_id
          AND retention.deployment_id=closure.deployment_id AND retention.resource_name=closure.resource_name
         JOIN recipe_deployment_resources resource ON resource.deployment_id=closure.deployment_id
          AND resource.resource_name=closure.resource_name AND resource.project_id=closure.project_id
          AND resource.resource_id=volume.id AND resource.resource_kind='volume' AND resource.ownership='owned'
          AND resource.install_progress='ready' AND resource.removal_progress='retained' AND resource.active_attempt_id IS NULL
          AND recipe_effect_resource_state(resource)=retention.after_state
         JOIN recipe_effect_attempts original ON original.id=closure.attempt_id AND original.action='create'
          AND original.deployment_id=closure.deployment_id AND original.project_id=closure.project_id
          AND original.resource_name=closure.resource_name AND original.generation=resource.execution_generation
         JOIN recipe_deployment_commands command ON command.id=original.command_id
          AND command.operation='install' AND command.deployment_id=closure.deployment_id AND command.project_id=closure.project_id
         JOIN recipe_deployment_execution_preparations whole ON whole.deployment_id=closure.deployment_id
          AND whole.command_id=command.id AND whole.project_id=closure.project_id AND whole.version=1 AND whole.profile='runtime_named_v1'
          AND whole.intent_hash=command.input_hash AND whole.input_hash=sha256(whole.canonical_input)
         WHERE closure.volume_id=$1 AND closure.project_id=$2 AND closure.vm_provider_namespace=$3 AND closure.vm_host_id=$4
          AND fact.proof_json-'id'=ready.proof_json-'id'
          AND EXISTS(SELECT 1 FROM recipe_effect_verifications proof JOIN recipe_execution_transitions completed
            ON completed.verification_id=proof.id AND completed.attempt_id=original.id
            AND completed.command_id=original.command_id AND completed.deployment_id=original.deployment_id
            AND completed.project_id=original.project_id AND completed.resource_name=original.resource_name
            AND completed.kind IN ('completed','reconciled') AND completed.operation_id=proof.verification_attempt_id
            WHERE proof.attempt_id=original.id AND proof.recorded_by='hephaestus_worker' AND proof.outcome='applied'
             AND proof.action='create' AND proof.generation=original.generation AND proof.deployment_id=original.deployment_id
             AND proof.project_id=original.project_id AND proof.resource_name=original.resource_name
             AND proof.input_hash=original.input_hash AND proof.identity_json=original.claim_json->'identity'
             AND completed.after_state->>'install'='ready' AND completed.after_state->>'last_action'='create'
             AND completed.after_state->>'active_attempt' IS NULL
             AND (completed.after_state->>'generation')::bigint=original.generation)
         LIMIT 2")
        .bind(volume).bind(project).bind(namespace).bind(host).fetch_all(&mut **tx).await.map_err(repository_error)?;
    let [row]: [Origin; 1] = rows
        .try_into()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    Ok(row)
}

fn restore(
    row: &Origin,
    snapshot: &recipe_application::DeploymentSnapshot,
    name: &CapabilitySlotKey,
) -> Result<EffectClaim, DeploymentError> {
    let wire = &row.claim_json.0;
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.command_actor),
        RequestId::from_uuid(row.idempotency_id),
        DeploymentOperation::Install,
    )?;
    let plan = snapshot
        .intent
        .resources()
        .get(name)
        .ok_or(DeploymentError::Unavailable)?;
    let claim = EffectClaim {
        command,
        deployment_id: snapshot.intent.id(),
        resource: name.clone(),
        identity: plan.identity(),
        input_hash: plan.input_hash(),
        resource_version: wire.resource_version,
        generation: wire.generation,
        action: ResourceAction::Create,
        provenance: AttemptProvenance {
            attempt_id: DeploymentAttemptId::from_uuid(wire.attempt_id)?,
            actor_id: UserId::from_uuid(wire.actor_id),
            request_id: RequestId::from_uuid(wire.request_id),
        },
    };
    if ClaimWire::new(&claim)? != *wire
        || wire.actor_id != command.actor_id().as_uuid()
        || wire.request_id.is_nil()
        || wire.resource_version == 0
        || wire.generation == 0
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(claim)
}
