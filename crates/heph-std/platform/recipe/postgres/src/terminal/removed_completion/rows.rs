//! Exact result/finished/admission linkage; original actors remain comparison data.
use crate::{execution_transitions, hydration, receipts, repository_error, rows::ReceiptRow};
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, CommandReceipt, DeploymentError, DeploymentLifecycle, DeploymentOperation,
    DeploymentSnapshot,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

#[derive(sqlx::FromRow)]
pub(super) struct Finished {
    #[sqlx(flatten)]
    pub receipt: ReceiptRow,
    pub finish_fingerprint: Vec<u8>,
    pub transition_id: Uuid,
    pub actor_id: Uuid,
    pub request_id: Uuid,
    pub command_actor: Option<Uuid>,
    pub idempotency_id: Option<Uuid>,
    pub command_input: Option<Vec<u8>>,
    pub before_version: Option<i64>,
    pub after_version: Option<i64>,
    pub evidence_valid: bool,
}

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
) -> Result<Vec<Finished>, DeploymentError> {
    let dangling: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM recipe_execution_transitions transition
        LEFT JOIN recipe_deployment_commands command ON command.id=transition.command_id
        LEFT JOIN recipe_command_results result ON result.transition_id=transition.id
        WHERE (transition.deployment_id=$1 OR command.deployment_id=$1) AND transition.kind='finished'
          AND (transition.after_lifecycle='removed' OR command.operation='remove' OR command.id IS NULL)
          AND result.command_id IS NULL)")
        .bind(snapshot.intent.id().as_uuid()).fetch_one(&mut **tx).await.map_err(repository_error)?;
    if dangling {
        return Err(DeploymentError::ReconciliationRequired);
    }
    sqlx::query_as("SELECT result.command_id id,result.deployment_id,result.project_id,result.input_hash,
        result.lifecycle receipt_lifecycle,result.version receipt_version,result.event_id,result.event_cursor,result.event_aggregate_version,
        result.finish_fingerprint,result.transition_id,result.actor_id,result.request_id,
        command.actor_id command_actor,command.idempotency_id,command.input_hash command_input,
        transition.before_deployment_version before_version,transition.after_deployment_version after_version,
        COALESCE(command.id=result.command_id AND command.operation='remove'
         AND command.deployment_id=result.deployment_id AND command.project_id=result.project_id
         AND command.input_hash=result.input_hash AND command.actor_id=result.actor_id
         AND command.first_request_id<>'00000000-0000-0000-0000-000000000000'::uuid
         AND command.receipt_lifecycle IN ('removing','recovery_required') AND command.receipt_version>0
         AND transition.command_id=result.command_id AND transition.operation_id=result.command_id
         AND transition.deployment_id=result.deployment_id AND transition.project_id=result.project_id
         AND transition.kind='finished' AND transition.attempt_id IS NULL AND transition.resource_name IS NULL
         AND transition.before_state IS NULL AND transition.after_state IS NULL AND transition.verification_id IS NULL
         AND transition.partial_retention_receipt_id IS NULL AND transition.ready_retention_fact_id IS NULL
         AND transition.before_lifecycle IN ('removing','recovery_required') AND transition.after_lifecycle=result.lifecycle
         AND transition.fingerprint=result.finish_fingerprint AND transition.event_id=result.event_id
         AND transition.event_cursor=result.event_cursor AND transition.event_aggregate_version=result.event_aggregate_version
         AND transition.actor_id=result.actor_id AND transition.request_id=result.request_id
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=result.event_id AND event.occurrence_id=transition.id AND event.scope_kind='project'
             AND event.scope_id=result.project_id AND event.aggregate_type='project' AND event.aggregate_id=result.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
             AND event.actor_type='user' AND event.actor_id=result.actor_id AND event.request_id=result.request_id
             AND event.cursor=result.event_cursor AND event.aggregate_version=result.event_aggregate_version)
         AND EXISTS(SELECT 1 FROM application_events event JOIN product_event_outbox outbox ON outbox.event_id=event.id
             WHERE event.id=command.event_id AND event.occurrence_id=command.idempotency_id AND event.scope_kind='project'
             AND event.scope_id=command.project_id AND event.aggregate_type='project' AND event.aggregate_id=command.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated' AND event.schema_version=1
             AND event.actor_type='user' AND event.actor_id=command.actor_id AND event.request_id=command.first_request_id
             AND event.cursor=command.event_cursor AND event.aggregate_version=command.event_aggregate_version),false) evidence_valid
        FROM recipe_command_results result
        LEFT JOIN recipe_deployment_commands command ON command.id=result.command_id
        LEFT JOIN recipe_execution_transitions transition ON transition.id=result.transition_id
        WHERE (result.deployment_id=$1 OR command.deployment_id=$1)
          AND (result.lifecycle='removed' OR command.operation='remove' OR command.id IS NULL)
        ORDER BY result.command_id")
        .bind(snapshot.intent.id().as_uuid()).fetch_all(&mut **tx).await.map_err(repository_error)
}

pub(super) const fn select<'a>(
    rows: &'a [Finished],
    snapshot: &DeploymentSnapshot,
) -> Result<Option<&'a Finished>, DeploymentError> {
    match (snapshot.lifecycle, rows) {
        (DeploymentLifecycle::Removed, [row]) => Ok(Some(row)),
        (DeploymentLifecycle::Removed, _) => Err(DeploymentError::ReconciliationRequired),
        (_, []) => Ok(None),
        _ => Err(DeploymentError::ReconciliationRequired),
    }
}

pub(super) fn restore(
    row: &Finished,
    snapshot: &DeploymentSnapshot,
) -> Result<(CommandReceipt, u64), DeploymentError> {
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(
            row.command_actor
                .ok_or(DeploymentError::ReconciliationRequired)?,
        ),
        RequestId::from_uuid(
            row.idempotency_id
                .ok_or(DeploymentError::ReconciliationRequired)?,
        ),
        DeploymentOperation::Remove,
    )?;
    let before = hydration::unsigned(
        row.before_version
            .ok_or(DeploymentError::ReconciliationRequired)?,
    )?;
    let after = hydration::unsigned(
        row.after_version
            .ok_or(DeploymentError::ReconciliationRequired)?,
    )?;
    let fingerprint = serde_json::to_vec(&(command, snapshot.intent.id(), before))
        .map(|bytes| ContentHash::digest(&bytes))
        .map_err(|_| DeploymentError::Serialization)?;
    if snapshot.lifecycle != DeploymentLifecycle::Removed
        || !row.evidence_valid
        || before == 0
        || before.checked_add(1) != Some(after)
        || after != snapshot.version
        || u64::try_from(row.receipt.receipt_version).ok() != Some(after)
        || row.receipt.receipt_lifecycle != "removed"
        || row.actor_id != command.actor_id().as_uuid()
        || row.request_id.is_nil()
        || row.receipt.event_id.is_nil()
        || row.receipt.event_cursor <= 0
        || row.receipt.event_aggregate_version <= 0
        || row.transition_id
            != execution_transitions::transition_id(command.id().as_uuid(), "finished")
        || row.finish_fingerprint.as_slice() != fingerprint.as_bytes()
        || row.command_input.as_deref() != Some(row.receipt.input_hash.as_slice())
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let input = hydration::digest(&row.receipt.input_hash)?;
    receipts::restore(&row.receipt, command, snapshot, input).map(|receipt| (receipt, before))
}
