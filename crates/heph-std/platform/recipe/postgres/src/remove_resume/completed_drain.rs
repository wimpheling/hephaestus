//! Exact durable completed Drain observation under a fresh current manager.

#[path = "completed_drain/closure.rs"]
mod closure;

use crate::{
    PostgresDeploymentRepository, effect_context, effect_state::State,
    execution_preparation::active_effect::hydration as original, execution_rows::ClaimWire,
    execution_transitions, repository_error, verification,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    AttemptProvenance, CommandIdentity, DeploymentAttemptId, DeploymentError, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, EffectClaim, ExistingInstanceClosureRequest,
    RemovalProgress, ResourceAction, ResourceProgress,
};
use serde_json::{Value, json};
use sqlx::{FromRow, Postgres, Transaction};
use uuid::Uuid;

impl PostgresDeploymentRepository {
    /// Observes an exact already completed original instance Drain under current authority.
    ///
    /// The configured fresh Remove admission, original closure, full current state,
    /// completed transition, worker verification and committed product event are
    /// independently compared. This performs no completion or provider IO. Returned
    /// progress is durable comparison data, never present physical cleanup proof.
    ///
    /// # Errors
    /// Rejects missing admission, lost current rights, changed originals, active or
    /// later-generation work, or any missing/contradictory completion evidence.
    pub async fn observe_completed_instance_drain(
        &self,
        identity: &AuthenticatedIdentity,
        request: &ExistingInstanceClosureRequest,
    ) -> Result<ResourceProgress, DeploymentError> {
        let config = self
            .remove_resume
            .as_ref()
            .ok_or(DeploymentError::InvalidAction)?;
        let mut context = self
            .execution_context(identity, request.command(), request.intent().id(), true)
            .await?;
        let result = async {
            if context.snapshot.intent != *request.intent() {
                return Err(DeploymentError::IntentMismatch);
            }
            let target = request.target();
            if target.provider_namespace() != config.namespace()
                || target.host_id() != config.host()
                || target.prepared().source().platform() != config.platform()
            {
                return Err(DeploymentError::IntentMismatch);
            }
            compare_closure(&mut context.tx, request, config).await?;
            let claim = &request.expected().drain;
            let plan = context
                .snapshot
                .intent
                .resources()
                .get(&claim.resource)
                .ok_or(DeploymentError::IntentMismatch)?;
            let row = original::row(&mut context.tx, claim.provenance.attempt_id.as_uuid()).await?;
            if original::restore(&row, &context.snapshot, plan, &claim.resource)? != *claim {
                return Err(DeploymentError::ReconciliationRequired);
            }
            let state =
                effect_context::state(&mut context.tx, claim.deployment_id, &claim.resource)
                    .await?;
            original::current_state(&mut context.tx, &context.snapshot, &claim.resource, &state)
                .await?;
            let rows = completions(&mut context.tx, claim).await?;
            if rows.len() != 1 {
                return Err(DeploymentError::ReconciliationRequired);
            }
            let completed = &rows[0];
            validate(completed, claim, &context.snapshot, &state)?;
            let operation = DeploymentAttemptId::from_uuid(completed.operation_id)?;
            let proof =
                verification::matching(&mut context.tx, claim, operation, "applied", None).await?;
            if completed.verification_id != Some(proof) {
                return Err(DeploymentError::ReconciliationRequired);
            }
            state.progress()
        }
        .await;
        let rollback = context.tx.rollback().await.map_err(repository_error);
        match result {
            Err(primary) => Err(primary),
            Ok(progress) => rollback.map(|()| progress),
        }
    }
}

async fn compare_closure(
    tx: &mut Transaction<'_, Postgres>,
    request: &ExistingInstanceClosureRequest,
    config: &recipe_application::RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let target = request.target();
    let create = target.original();
    let actual: Value = sqlx::query_scalar(
        "SELECT load_existing_instance_closure118($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)",
    )
    .bind(request.command().id().as_uuid())
    .bind(request.intent().id().as_uuid())
    .bind(request.intent().project_id().as_uuid())
    .bind(request.intent().input_hash().as_bytes().as_slice())
    .bind(create.provenance.attempt_id.as_uuid())
    .bind(
        serde_json::to_value(ClaimWire::new(create)?)
            .map_err(|_| DeploymentError::Serialization)?,
    )
    .bind(serde_json::to_value(target.prepared()).map_err(|_| DeploymentError::Serialization)?)
    .bind(config.namespace())
    .bind(config.host())
    .bind(serde_json::to_value(config.platform()).map_err(|_| DeploymentError::Serialization)?)
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    closure::compare(&actual, request)
}

#[derive(Clone, FromRow)]
struct Completion {
    id: Uuid,
    command_id: Uuid,
    operation_id: Uuid,
    kind: String,
    fingerprint: Vec<u8>,
    actor_id: Uuid,
    request_id: Uuid,
    command_actor: Uuid,
    idempotency_id: Uuid,
    command_operation: String,
    before_state: sqlx::types::Json<Value>,
    after_state: sqlx::types::Json<Value>,
    before_deployment_version: i64,
    after_deployment_version: i64,
    before_lifecycle: String,
    after_lifecycle: String,
    verification_id: Option<Uuid>,
    evidence_valid: bool,
}

async fn completions(
    tx: &mut Transaction<'_, Postgres>,
    claim: &EffectClaim,
) -> Result<Vec<Completion>, DeploymentError> {
    sqlx::query_as(
        "SELECT t.id,t.command_id,t.operation_id,t.kind,t.fingerprint,t.actor_id,t.request_id,
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
             WHERE event.id=t.event_id AND event.scope_kind='project' AND event.scope_id=t.project_id
             AND event.aggregate_type='project' AND event.aggregate_id=t.project_id AND event.event_type='project.changed'
             AND event.change_kind='updated' AND event.schema_version=1 AND event.occurrence_id=t.id
             AND event.actor_type='user' AND event.actor_id=t.actor_id
             AND event.request_id=t.request_id AND event.cursor=t.event_cursor AND event.aggregate_version=t.event_aggregate_version)) evidence_valid
         FROM recipe_execution_transitions t JOIN recipe_effect_attempts attempt ON attempt.id=t.attempt_id
         JOIN recipe_deployment_commands command ON command.id=t.command_id
         WHERE t.attempt_id=$1 AND t.deployment_id=$2 AND t.resource_name=$3 AND t.kind IN ('completed','reconciled')
           AND t.after_state->>'active_attempt' IS NULL AND t.after_state->>'last_action'='drain'
           AND t.after_state->>'removal'='draining' AND t.after_state->>'diagnostic' IS NULL",
    )
    .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.deployment_id.as_uuid()).bind(claim.resource.as_str())
    .fetch_all(&mut **tx).await.map_err(repository_error)
}

fn validate(
    completed: &Completion,
    claim: &EffectClaim,
    snapshot: &DeploymentSnapshot,
    current: &State,
) -> Result<(), DeploymentError> {
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(completed.command_actor),
        RequestId::from_uuid(completed.idempotency_id),
        completed.command_operation.parse()?,
    )?;
    let before = decode_state(&completed.before_state.0)?;
    let operation = DeploymentAttemptId::from_uuid(completed.operation_id)?;
    let provenance = AttemptProvenance {
        attempt_id: operation,
        actor_id: command.actor_id(),
        request_id: RequestId::from_uuid(completed.request_id),
    };
    let fingerprint = match completed.kind.as_str() {
        "completed"
            if command == claim.command
                && operation == claim.provenance.attempt_id
                && completed.request_id == claim.provenance.request_id.as_uuid()
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
    let before_lifecycle: DeploymentLifecycle = completed.before_lifecycle.parse()?;
    let after_lifecycle: DeploymentLifecycle = completed.after_lifecycle.parse()?;
    let mut expected = before.applied(ResourceAction::Drain);
    expected.version = before
        .version
        .checked_add(1)
        .ok_or(DeploymentError::StaleClaim)?;
    if claim.action != ResourceAction::Drain
        || command.operation() != DeploymentOperation::Remove
        || command.id().as_uuid() != completed.command_id
        || command.actor_id().as_uuid() != completed.actor_id
        || completed.request_id.is_nil()
        || completed.id
            != execution_transitions::transition_id(completed.operation_id, &completed.kind)
        || completed.fingerprint.as_slice() != fingerprint.as_bytes()
        || !completed.evidence_valid
        || before.active_attempt != Some(claim.provenance.attempt_id.as_uuid())
        || u64::try_from(before.generation).ok() != Some(claim.generation)
        || u64::try_from(before.version)
            .ok()
            .is_none_or(|version| version < claim.resource_version)
        || !matches!(
            before_lifecycle,
            DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
        )
        || !matches!(
            after_lifecycle,
            DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
        )
        || completed.before_deployment_version < 0
        || completed.before_deployment_version.checked_add(1)
            != Some(completed.after_deployment_version)
        || u64::try_from(completed.after_deployment_version)
            .ok()
            .is_none_or(|version| version > snapshot.version)
        || completed.after_state.0
            != serde_json::to_value(&expected).map_err(|_| DeploymentError::Serialization)?
        || current != &expected
        || completed.verification_id.is_none()
        || current.active_attempt.is_some()
        || current.removal != RemovalProgress::Draining
        || current.last_action != Some(ResourceAction::Drain)
        || current.diagnostic.is_some()
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

fn decode_state(value: &Value) -> Result<State, DeploymentError> {
    let state: State = serde_json::from_value(value.clone())
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    if serde_json::to_value(&state).map_err(|_| DeploymentError::Serialization)? != *value {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(state)
}

#[cfg(test)]
#[path = "completed_drain_tests.rs"]
mod tests;
