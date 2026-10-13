use crate::{
    effect_context::Context, effect_state::State, execution_rows::ClaimWire, execution_transitions,
    repository_error,
};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentError, DiagnosticCode, EffectClaim,
    InstallProgress, ResourceAction, ResourceProgress, RetainedExternalCompletion,
};
use serde_json::json;
use uuid::Uuid;

#[derive(sqlx::FromRow)]
struct Completion {
    id: Uuid,
    kind: String,
    operation_id: Uuid,
    fingerprint: Vec<u8>,
    actor_id: Uuid,
    request_id: Uuid,
    verification_attempt_id: Uuid,
    after_state: sqlx::types::Json<State>,
    qualified: bool,
}

pub(super) struct Completed {
    row: Completion,
    pub(super) progress: ResourceProgress,
}

impl Completed {
    pub(super) fn matches(
        &self,
        claim: &EffectClaim,
        requested: RetainedExternalCompletion,
    ) -> Result<(), DeploymentError> {
        let (kind, operation, input) = input(claim, requested);
        if self.row.kind != kind
            || self.row.operation_id != operation
            || self.row.fingerprint.as_slice()
                != execution_transitions::fingerprint(claim, kind, &input)?.as_bytes()
        {
            return Err(DeploymentError::InputConflict);
        }
        Ok(())
    }
}

pub(super) fn reopening_id(claim: &EffectClaim) -> Uuid {
    Uuid::new_v5(
        &claim.provenance.attempt_id.as_uuid(),
        b"heph-owned-ready-reopening-v1\0",
    )
}

pub(super) async fn load(
    context: &mut Context<'_>,
    state: &State,
    claim: &EffectClaim,
    closure: Uuid,
) -> Result<Option<Completed>, DeploymentError> {
    let reopened = reopening(context, claim, closure).await?;
    let rows: Vec<Completion> = sqlx::query_as(
        "SELECT transition.id,transition.kind,transition.operation_id,transition.fingerprint,
          transition.actor_id,transition.request_id,proof.verification_attempt_id,transition.after_state,
          (transition.command_id=$2 AND transition.deployment_id=$3 AND transition.project_id=$4
           AND transition.resource_name=$5 AND transition.actor_id=$6 AND transition.request_id<>'00000000-0000-0000-0000-000000000000'
           AND transition.after_deployment_version<=$7 AND proof.recorded_by='hephaestus_worker'
           AND proof.deployment_id=$3 AND proof.project_id=$4 AND proof.resource_name=$5
           AND proof.generation=$8 AND proof.action='verify_external' AND proof.identity_json=$9
           AND proof.input_hash=$10 AND proof.diagnostic IS NULL
           AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=transition.event_id AND event.occurrence_id=transition.id
              AND event.scope_kind='project' AND event.scope_id=$4 AND event.aggregate_type='project'
              AND event.aggregate_id=$4 AND event.event_type='project.changed' AND event.actor_id=transition.actor_id
              AND event.request_id=transition.request_id AND event.cursor=transition.event_cursor
              AND event.aggregate_version=transition.event_aggregate_version)) AS qualified
         FROM recipe_execution_transitions transition JOIN recipe_effect_verifications proof
          ON proof.id=transition.verification_id AND proof.attempt_id=transition.attempt_id
         WHERE transition.attempt_id=$1 AND transition.kind IN ('completed','reconciled') AND proof.outcome='applied' LIMIT 2")
        .bind(claim.provenance.attempt_id.as_uuid()).bind(claim.command.id().as_uuid())
        .bind(claim.deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
        .bind(claim.resource.as_str()).bind(claim.command.actor_id().as_uuid())
        .bind(i64::try_from(context.snapshot.version).map_err(|_| DeploymentError::StaleClaim)?)
        .bind(i64::try_from(claim.generation).map_err(|_| DeploymentError::StaleClaim)?)
        .bind(serde_json::to_value(claim.identity).map_err(|_| DeploymentError::Serialization)?)
        .bind(claim.input_hash.as_bytes().as_slice()).fetch_all(&mut *context.tx).await.map_err(repository_error)?;
    if !reopened && rows.is_empty() {
        if state.install == InstallProgress::Ready || state.active_attempt.is_none() {
            return Err(DeploymentError::ReconciliationRequired);
        }
        return Ok(None);
    }
    if !reopened {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let [row]: [Completion; 1] = rows
        .try_into()
        .map_err(|_| DeploymentError::ReconciliationRequired)?;
    validate(&row, state, claim)?;
    let progress = state.progress()?;
    Ok(Some(Completed { row, progress }))
}

async fn reopening(
    context: &mut Context<'_>,
    claim: &EffectClaim,
    closure: Uuid,
) -> Result<bool, DeploymentError> {
    let wire =
        serde_json::to_value(ClaimWire::new(claim)?).map_err(|_| DeploymentError::Serialization)?;
    let rows: Vec<bool> = sqlx::query_scalar(
        "SELECT (reopening.id=$2 AND reopening.closure_id=$1 AND reopening.attempt_id=$3
          AND reopening.deployment_id=$4 AND reopening.project_id=$5 AND reopening.resource_name=$6
          AND reopening.command_id=$7 AND reopening.actor_id=$8 AND reopening.claim_json=$9
          AND volume.ready_retention_closure_id=$1 AND volume.ready_retention_reopen_id=reopening.id
          AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
            WHERE event.id=reopening.event_id AND event.occurrence_id=reopening.id
             AND event.scope_kind='project' AND event.scope_id=$5 AND event.aggregate_type='project'
             AND event.aggregate_id=$5 AND event.event_type='project.changed' AND event.actor_id=reopening.actor_id
             AND event.request_id=reopening.request_id))
         FROM recipe_owned_ready_reopenings reopening JOIN volume_owned_ready_retention_closures closure ON closure.id=reopening.closure_id
          JOIN agent_instance_state_volumes volume ON volume.id=closure.volume_id
         WHERE reopening.closure_id=$1 OR reopening.id=$2 OR reopening.attempt_id=$3 LIMIT 2")
        .bind(closure).bind(reopening_id(claim)).bind(claim.provenance.attempt_id.as_uuid())
        .bind(claim.deployment_id.as_uuid()).bind(context.snapshot.intent.project_id().as_uuid())
        .bind(claim.resource.as_str()).bind(claim.command.id().as_uuid()).bind(claim.command.actor_id().as_uuid())
        .bind(wire).fetch_all(&mut *context.tx).await.map_err(repository_error)?;
    match rows.as_slice() {
        [] => Ok(false),
        [true] => Ok(true),
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}

fn validate(row: &Completion, state: &State, claim: &EffectClaim) -> Result<(), DeploymentError> {
    if !row.qualified
        || row.after_state.0 != *state
        || state.install != InstallProgress::Ready
        || state.active_attempt.is_some()
        || state.last_action != Some(ResourceAction::VerifyExternal)
        || row.operation_id != row.verification_attempt_id
        || row.id != execution_transitions::transition_id(row.operation_id, &row.kind)
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let requested = match row.kind.as_str() {
        "completed" if row.operation_id == claim.provenance.attempt_id.as_uuid() => {
            RetainedExternalCompletion::Completed
        }
        "reconciled" if row.operation_id != claim.provenance.attempt_id.as_uuid() => {
            RetainedExternalCompletion::Reconciled(AttemptProvenance {
                attempt_id: DeploymentAttemptId::from_uuid(row.operation_id)?,
                actor_id: UserId::from_uuid(row.actor_id),
                request_id: RequestId::from_uuid(row.request_id),
            })
        }
        _ => return Err(DeploymentError::ReconciliationRequired),
    };
    let (kind, _, input) = input(claim, requested);
    if row.fingerprint.as_slice()
        != execution_transitions::fingerprint(claim, kind, &input)?.as_bytes()
    {
        return Err(DeploymentError::InputConflict);
    }
    Ok(())
}

fn input(
    claim: &EffectClaim,
    requested: RetainedExternalCompletion,
) -> (&'static str, Uuid, serde_json::Value) {
    let diagnostic = Option::<DiagnosticCode>::None;
    match requested {
        RetainedExternalCompletion::Completed => (
            "completed",
            claim.provenance.attempt_id.as_uuid(),
            json!(["applied", diagnostic]),
        ),
        RetainedExternalCompletion::Reconciled(provenance) => (
            "reconciled",
            provenance.attempt_id.as_uuid(),
            json!([claim.command, provenance, "applied", diagnostic]),
        ),
    }
}
