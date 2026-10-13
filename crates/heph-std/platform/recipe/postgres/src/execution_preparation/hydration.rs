use event_application::CommittedMutation;
use identity_domain::{RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentExecutionPreparation, DeploymentIntent,
    DeploymentOperation, ExecutionPreparationAdmission, ExecutionPreparationReceipt,
    PreparedInstanceImport,
};
use release_domain::ContentHash;
use sqlx::{Postgres, Transaction};

use crate::{hydration::digest, repository_error};

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DeploymentIntent,
) -> Result<Option<ExecutionPreparationAdmission>, DeploymentError> {
    let row: Option<super::rows::HeaderRow> = sqlx::query_as(
        "SELECT header.project_id,header.command_id,header.actor_id,command.actor_id AS original_actor_id,
           command.idempotency_id,command.operation,header.version,header.profile,header.intent_hash,
           header.platform_json,header.instance_count,header.canonical_input,header.input_hash,
           header.request_id,header.event_id,header.event_cursor,header.event_aggregate_version,
           EXISTS(SELECT 1 FROM application_events event WHERE event.id=header.event_id
             AND event.scope_kind='project' AND event.scope_id=header.project_id
             AND event.aggregate_type='project' AND event.aggregate_id=header.project_id
             AND event.event_type='project.changed' AND event.change_kind='updated'
             AND event.actor_type='user' AND event.actor_id=header.actor_id AND event.request_id=header.request_id
             AND event.cursor=header.event_cursor AND event.aggregate_version=header.event_aggregate_version
             AND event.related_id_one=(SELECT organization_id FROM projects WHERE id=header.project_id)
             AND event.related_id_two IS NULL) AS event_valid
         FROM recipe_deployment_execution_preparations header JOIN recipe_deployment_commands command
           ON command.id=header.command_id AND command.deployment_id=header.deployment_id AND command.project_id=header.project_id
         WHERE header.deployment_id=$1",
    )
    .bind(intent.id().as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?;
    let Some(row) = row else {
        return Ok(None);
    };
    let result = restore(intent, &row)?;
    let children: Vec<super::rows::ImportRow> = sqlx::query_as(
        "SELECT resource_name,ordinal,operation_id,import_command_key,instance_id,revision_id,
           release_id,release_agent_id,canonical_import,input_hash
         FROM recipe_prepared_instance_imports WHERE deployment_id=$1 ORDER BY ordinal LIMIT 65",
    )
    .bind(intent.id().as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(repository_error)?;
    if children.len() != result.preparation.instances().len() {
        return Err(DeploymentError::IntentMismatch);
    }
    for (ordinal, (child, prepared)) in children
        .iter()
        .zip(result.preparation.instances())
        .enumerate()
    {
        validate_child(ordinal, child, prepared)?;
    }
    Ok(Some(result))
}

fn restore(
    intent: &DeploymentIntent,
    row: &super::rows::HeaderRow,
) -> Result<ExecutionPreparationAdmission, DeploymentError> {
    let operation: DeploymentOperation = row.operation.parse()?;
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.actor_id),
        RequestId::from_uuid(row.idempotency_id),
        operation,
    )?;
    let hash = digest(&row.input_hash)?;
    let preparation = DeploymentExecutionPreparation::from_canonical_bytes(
        intent,
        command,
        &row.canonical_input,
        hash,
    )?;
    if row.version != 1
        || row.profile != "runtime_named_v1"
        || row.project_id != intent.project_id().as_uuid()
        || row.actor_id != row.original_actor_id
        || row.command_id != command.id().as_uuid()
        || row.intent_hash.as_slice() != intent.input_hash().as_bytes()
        || row.platform_json
            != serde_json::to_value(preparation.platform())
                .map_err(|_| DeploymentError::Serialization)?
        || usize::try_from(row.instance_count).ok() != Some(preparation.instances().len())
        || row.event_id.is_nil()
        || row.request_id.is_nil()
        || row.event_cursor <= 0
        || row.event_aggregate_version <= 0
        || !row.event_valid
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(ExecutionPreparationAdmission {
        preparation,
        receipt: ExecutionPreparationReceipt {
            command,
            input_hash: hash,
            preparation_request_id: RequestId::from_uuid(row.request_id),
            event: CommittedMutation {
                event_id: row.event_id,
                scope_kind: String::from("project"),
                scope_id: row.project_id,
                cursor: row.event_cursor,
                aggregate_version: row.event_aggregate_version,
            },
        },
        replayed: true,
    })
}

fn validate_child(
    ordinal: usize,
    row: &super::rows::ImportRow,
    prepared: &PreparedInstanceImport,
) -> Result<(), DeploymentError> {
    let canonical = serde_json::to_vec(prepared).map_err(|_| DeploymentError::Serialization)?;
    if usize::try_from(row.ordinal).ok() != Some(ordinal)
        || row.resource_name != prepared.resource().as_str()
        || row.operation_id != prepared.operation_id()
        || row.import_command_key.as_slice() != prepared.command_key().as_bytes()
        || row.instance_id != prepared.instance_id().as_uuid()
        || row.revision_id != prepared.revision_id().as_uuid()
        || row.release_id != prepared.release_pin().release_id.as_uuid()
        || row.release_agent_id != prepared.release_pin().release_agent_id.as_uuid()
        || row.canonical_import != canonical
        || digest(&row.input_hash)? != ContentHash::digest(&canonical)
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
