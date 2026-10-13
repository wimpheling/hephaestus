use super::ledger;
use crate::{effect_state::State, execution_transitions, repository_error};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentOperation, DeploymentSnapshot,
    EffectClaim, InstallProgress, RemovalProgress, RemoveResumeConfiguration, ResourceAction,
    RetainOwnedReady,
};
use serde_json::Value;
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(FromRow)]
pub(super) struct Retention {
    pub id: Uuid,
    pub command_id: Uuid,
    pub operation_id: Uuid,
    pub fingerprint: Vec<u8>,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub command_actor: Uuid,
    pub idempotency_id: Uuid,
    pub before_state: sqlx::types::Json<Value>,
    pub after_state: sqlx::types::Json<Value>,
    pub before_deployment_version: i64,
    pub after_deployment_version: i64,
    pub before_lifecycle: String,
    pub after_lifecycle: String,
    pub evidence_valid: bool,
}

pub(super) async fn qualify(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
    claim: &EffectClaim,
    ready: &State,
    current: &State,
    config: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let rows: Vec<Retention> = sqlx::query_as("SELECT transition.id,transition.command_id,transition.operation_id,
        transition.fingerprint,transition.actor_id,transition.request_id,command.actor_id command_actor,command.idempotency_id,
        transition.before_state,transition.after_state,transition.before_deployment_version,transition.after_deployment_version,
        transition.before_lifecycle,transition.after_lifecycle,
        (command.operation='remove' AND command.deployment_id=transition.deployment_id AND command.project_id=transition.project_id
         AND transition.verification_id IS NULL AND transition.ready_retention_fact_id=fact.id
         AND closure.command_id=transition.command_id AND closure.operation_id=transition.operation_id
         AND closure.fingerprint=transition.fingerprint AND closure.expected_deployment_version=transition.before_deployment_version
         AND closure.expected_resource_version=(transition.before_state->>'version')::bigint
         AND closure.actor_id=transition.actor_id AND closure.request_id=transition.request_id
         AND closure.vm_provider_namespace=$5 AND closure.vm_host_id=$6
         AND closure.id=volume.ready_retention_closure_id AND volume.ready_retention_reopen_id IS NULL
         AND closure.volume_id=resource.resource_id AND closure.project_id=transition.project_id
         AND closure.deployment_id=transition.deployment_id AND closure.resource_name=transition.resource_name
         AND closure.seal_id=seal.id AND closure.creation_operation_id=seal.operation_id
         AND closure.attempt_id=seal.attempt_id AND closure.generation=1
         AND closure.provisioning_operation_id=observed.operation_id AND closure.purpose_hash=purpose.purpose_hash
         AND observed.id=closure.ready_observation_id AND observed.volume_id=volume.id AND observed.generation=closure.generation
         AND fact.evidence_hash=sha256(convert_to(fact.proof_json::text,'UTF8'))
         AND fact.proof_json->>'id'=fact.id::text AND fact.proof_json-'id'=observed.proof_json-'id'
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=closure.event_id AND event.occurrence_id=closure.id AND event.scope_kind='project'
             AND event.scope_id=closure.project_id AND event.aggregate_type='project' AND event.aggregate_id=closure.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
             AND event.actor_type='user' AND event.actor_id=closure.actor_id AND event.request_id=closure.request_id
             AND event.cursor=closure.event_cursor AND event.aggregate_version=closure.event_aggregate_version)
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=fact.event_id AND event.occurrence_id=fact.id AND event.scope_kind='project'
             AND event.scope_id=closure.project_id AND event.aggregate_type='project' AND event.aggregate_id=closure.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
             AND event.actor_type='system' AND event.actor_id IS NULL)
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=transition.event_id AND event.occurrence_id=transition.id AND event.scope_kind='project'
             AND event.scope_id=transition.project_id AND event.aggregate_type='project' AND event.aggregate_id=transition.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
             AND event.actor_type='user' AND event.actor_id=transition.actor_id AND event.request_id=transition.request_id
             AND event.cursor=transition.event_cursor AND event.aggregate_version=transition.event_aggregate_version)) evidence_valid
        FROM recipe_execution_transitions transition
        JOIN recipe_deployment_commands command ON command.id=transition.command_id
        JOIN recipe_deployment_resources resource ON resource.deployment_id=transition.deployment_id AND resource.resource_name=transition.resource_name
        JOIN agent_instance_state_volumes volume ON volume.id=resource.resource_id
        JOIN recipe_owned_ready_retentions retained ON retained.transition_id=transition.id
        JOIN volume_owned_ready_retention_closures closure ON closure.id=retained.closure_id
        JOIN volume_owned_ready_retention_facts fact ON fact.id=retained.fact_id AND fact.closure_id=closure.id
        JOIN volume_creation_seals seal ON seal.volume_id=volume.id
        JOIN volume_owned_backing_purposes purpose ON purpose.volume_id=volume.id
        JOIN volume_owned_backing_observations observed ON observed.id=closure.ready_observation_id
        WHERE transition.attempt_id=$1 AND transition.deployment_id=$2 AND transition.project_id=$3
          AND transition.resource_name=$4 AND transition.kind='creation_retained_ready'")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid())
        .bind(snapshot.intent.project_id().as_uuid()).bind(claim.resource.as_str())
        .bind(config.namespace()).bind(config.host())
        .fetch_all(&mut **tx).await.map_err(repository_error)?;
    let [row] = rows.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    validate(row, claim, ready, current, snapshot.version)
}

pub(super) fn validate(
    row: &Retention,
    claim: &EffectClaim,
    ready: &State,
    current: &State,
    version: u64,
) -> Result<(), DeploymentError> {
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.command_actor),
        RequestId::from_uuid(row.idempotency_id),
        DeploymentOperation::Remove,
    )?;
    let before = ledger::decode(&row.before_state.0)?;
    let request = RetainOwnedReady {
        command,
        target: claim.clone(),
        operation_id: DeploymentAttemptId::from_uuid(row.operation_id)?,
        expected_deployment_version: u64::try_from(row.before_deployment_version)
            .map_err(|_| DeploymentError::ReconciliationRequired)?,
        expected_resource_version: u64::try_from(before.version)
            .map_err(|_| DeploymentError::ReconciliationRequired)?,
    };
    let mut after = before.clone();
    after.install = InstallProgress::Ready;
    after.removal = RemovalProgress::Retained;
    after.active_attempt = None;
    after.diagnostic = None;
    after.last_action = Some(ResourceAction::Retain);
    after.version = before
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    if before != *ready
        || !row.evidence_valid
        || row.request_id.is_nil()
        || command.id().as_uuid() != row.command_id
        || command.actor_id().as_uuid() != row.actor_id
        || row.operation_id == claim.provenance.attempt_id.as_uuid()
        || row.id
            != execution_transitions::transition_id(row.operation_id, "creation_retained_ready")
        || row.fingerprint.as_slice()
            != execution_transitions::fingerprint(
                claim,
                "creation_retained_ready",
                &serde_json::json!([
                    request.command,
                    request.operation_id,
                    request.expected_deployment_version,
                    request.expected_resource_version
                ]),
            )?
            .as_bytes()
        || !matches!(
            row.before_lifecycle.as_str(),
            "removing" | "recovery_required"
        )
        || !matches!(
            row.after_lifecycle.as_str(),
            "removing" | "recovery_required"
        )
        || row.before_deployment_version.checked_add(1) != Some(row.after_deployment_version)
        || u64::try_from(row.after_deployment_version)
            .ok()
            .is_none_or(|value| value > version)
        || ledger::decode(&row.after_state.0)? != after
        || *current != after
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}
