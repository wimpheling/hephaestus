//! Positive Applied Create ledger; recorded actor fields confer no current rights.
use crate::{effect_state::State, execution_transitions, repository_error, verification};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentLifecycle,
    DeploymentOperation, EffectClaim, InstallProgress, RemovalProgress, ResourceAction,
};
use serde_json::{Value, json};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

#[derive(FromRow)]
pub(super) struct Completion {
    pub id: Uuid,
    pub command_id: Uuid,
    pub operation_id: Uuid,
    pub kind: String,
    pub fingerprint: Vec<u8>,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub command_actor: Uuid,
    pub idempotency_id: Uuid,
    pub command_operation: String,
    pub before_state: sqlx::types::Json<Value>,
    pub after_state: sqlx::types::Json<Value>,
    pub before_deployment_version: i64,
    pub after_deployment_version: i64,
    pub before_lifecycle: String,
    pub after_lifecycle: String,
    pub verification_id: Option<Uuid>,
    pub evidence_valid: bool,
}

pub(super) async fn command_event(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
) -> Result<(), DeploymentError> {
    let exact: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_deployment_commands command
        JOIN application_events event ON event.id=command.event_id
        JOIN product_event_outbox outbox ON outbox.event_id=event.id WHERE command.id=$1
        AND event.occurrence_id=command.idempotency_id AND event.scope_kind='project' AND event.scope_id=command.project_id
        AND event.aggregate_type='project' AND event.aggregate_id=command.project_id AND event.event_type='project.changed'
        AND event.change_kind='updated' AND event.schema_version=1 AND event.actor_type='user'
        AND event.actor_id=command.actor_id AND event.request_id=command.first_request_id
        AND event.cursor=command.event_cursor AND event.aggregate_version=command.event_aggregate_version")
        .bind(command.id().as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if exact {
        Ok(())
    } else {
        Err(DeploymentError::ReconciliationRequired)
    }
}

pub(super) async fn ready(
    tx: &mut Transaction<'_, Postgres>,
    claim: &EffectClaim,
    version: u64,
) -> Result<State, DeploymentError> {
    let rows: Vec<Completion> = sqlx::query_as("SELECT t.id,t.command_id,t.operation_id,t.kind,t.fingerprint,t.actor_id,t.request_id,
        command.actor_id command_actor,command.idempotency_id,command.operation command_operation,
        t.before_state,t.after_state,t.before_deployment_version,t.after_deployment_version,t.before_lifecycle,t.after_lifecycle,t.verification_id,
        (t.project_id=attempt.project_id AND t.deployment_id=attempt.deployment_id AND t.resource_name=attempt.resource_name
         AND command.deployment_id=t.deployment_id AND command.project_id=t.project_id
         AND t.before_deployment_version>attempt.expected_deployment_version
         AND EXISTS(SELECT 1 FROM recipe_effect_verifications proof WHERE proof.id=t.verification_id
           AND proof.attempt_id=t.attempt_id AND proof.verification_attempt_id=t.operation_id
           AND proof.deployment_id=t.deployment_id AND proof.project_id=t.project_id AND proof.resource_name=t.resource_name
           AND proof.generation=attempt.generation AND proof.action=attempt.action
           AND proof.identity_json=attempt.claim_json->'identity' AND proof.input_hash=attempt.input_hash
           AND proof.outcome='applied' AND proof.diagnostic IS NULL AND proof.recorded_by='hephaestus_worker')
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
           WHERE event.id=t.event_id AND event.occurrence_id=t.id AND event.scope_kind='project' AND event.scope_id=t.project_id
           AND event.aggregate_type='project' AND event.aggregate_id=t.project_id AND event.event_type='project.changed'
           AND event.change_kind='updated' AND event.schema_version=1 AND event.actor_type='user'
           AND event.actor_id=t.actor_id AND event.request_id=t.request_id
           AND event.cursor=t.event_cursor AND event.aggregate_version=t.event_aggregate_version)) evidence_valid
        FROM recipe_execution_transitions t JOIN recipe_effect_attempts attempt ON attempt.id=t.attempt_id
        JOIN recipe_deployment_commands command ON command.id=t.command_id
        WHERE t.attempt_id=$1 AND t.kind IN ('completed','reconciled')
          AND t.after_state->>'install'='ready' AND t.after_state->>'last_action'='create'
          AND t.after_state->>'active_attempt' IS NULL")
        .bind(claim.provenance.attempt_id.as_uuid()).fetch_all(&mut **tx).await.map_err(repository_error)?;
    let [completed] = rows.as_slice() else {
        return Err(DeploymentError::ReconciliationRequired);
    };
    let ready = validate(completed, claim, version)?;
    let proof = verification::matching(
        tx,
        claim,
        DeploymentAttemptId::from_uuid(completed.operation_id)?,
        "applied",
        None,
    )
    .await?;
    if completed.verification_id != Some(proof) {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(ready)
}

pub(super) fn validate(
    row: &Completion,
    claim: &EffectClaim,
    version: u64,
) -> Result<State, DeploymentError> {
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.command_actor),
        RequestId::from_uuid(row.idempotency_id),
        row.command_operation.parse()?,
    )?;
    let before = decode(&row.before_state.0)?;
    let operation = DeploymentAttemptId::from_uuid(row.operation_id)?;
    let provenance = AttemptProvenance {
        attempt_id: operation,
        actor_id: command.actor_id(),
        request_id: RequestId::from_uuid(row.request_id),
    };
    let fingerprint = match row.kind.as_str() {
        "completed"
            if command == claim.command
                && operation == claim.provenance.attempt_id
                && row.request_id == claim.provenance.request_id.as_uuid()
                && u64::try_from(before.version).ok() == Some(claim.resource_version) =>
        {
            execution_transitions::fingerprint(claim, "completed", &json!(["applied", null]))?
        }
        "reconciled" if operation != claim.provenance.attempt_id => {
            execution_transitions::fingerprint(
                claim,
                "reconciled",
                &json!([command, provenance, "applied", null]),
            )?
        }
        _ => return Err(DeploymentError::ReconciliationRequired),
    };
    let after = decode(&row.after_state.0)?;
    let mut expected = before.applied(ResourceAction::Create);
    expected.version = before
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    let before_lifecycle: DeploymentLifecycle = row.before_lifecycle.parse()?;
    let after_lifecycle: DeploymentLifecycle = row.after_lifecycle.parse()?;
    if claim.action != ResourceAction::Create
        || claim.command.operation() != DeploymentOperation::Install
        || !matches!(
            command.operation(),
            DeploymentOperation::Install | DeploymentOperation::Remove
        )
        || command.id().as_uuid() != row.command_id
        || command.actor_id().as_uuid() != row.actor_id
        || row.request_id.is_nil()
        || row.id != execution_transitions::transition_id(row.operation_id, &row.kind)
        || row.fingerprint.as_slice() != fingerprint.as_bytes()
        || !row.evidence_valid
        || before.active_attempt != Some(claim.provenance.attempt_id.as_uuid())
        || u64::try_from(before.generation).ok() != Some(claim.generation)
        || u64::try_from(before.version)
            .ok()
            .is_none_or(|value| value < claim.resource_version)
        || before.removal != RemovalProgress::Pending
        || !matches!(
            before.install,
            InstallProgress::Creating | InstallProgress::Failed | InstallProgress::RecoveryRequired
        )
        || !matches!(
            before_lifecycle,
            DeploymentLifecycle::Installing
                | DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
        )
        || !matches!(
            after_lifecycle,
            DeploymentLifecycle::Installing
                | DeploymentLifecycle::Removing
                | DeploymentLifecycle::RecoveryRequired
        )
        || row.before_deployment_version < 0
        || row.before_deployment_version.checked_add(1) != Some(row.after_deployment_version)
        || u64::try_from(row.after_deployment_version)
            .ok()
            .is_none_or(|value| value > version)
        || row.verification_id.is_none()
        || after != expected
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(after)
}

pub(super) fn decode(value: &Value) -> Result<State, DeploymentError> {
    let state: State = serde_json::from_value(value.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if serde_json::to_value(&state).map_err(|_| DeploymentError::Serialization)? != *value {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(state)
}
