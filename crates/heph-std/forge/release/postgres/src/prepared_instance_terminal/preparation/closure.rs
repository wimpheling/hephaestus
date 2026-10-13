//! Actual same-snapshot originals; no reconstruction of native removal/inbox rows.
use super::{
    super::{origin, transaction, wire},
    decode,
};
use crate::prepared_instance_safety;
use recipe_application::{
    DeploymentError, OriginalInstanceClosure, PreparedInstanceTerminalPreparationInput,
    ResourceAction,
};
use release_service::InstanceExecutionConfiguration;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub(super) async fn load(
    tx: &mut Transaction<'_, Postgres>,
    input: &PreparedInstanceTerminalPreparationInput,
    configuration: &InstanceExecutionConfiguration,
) -> Result<(OriginalInstanceClosure, Value), DeploymentError> {
    let context: Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
        .bind(input.intent().id().as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(|error| transaction::storage(&error))?;
    originals(tx, input, configuration).await?;
    // Delete uses the native143 current front below, which additionally admits
    // positively completed terminal replay. Do not narrow it to active claims.
    if input.effect().action == ResourceAction::Detach
        && input.current_command() != input.effect().command
    {
        let qualified: bool =
            sqlx::query_scalar("SELECT qualified_remove_resume_claim118($1,$2,$3,$4,$5)")
                .bind(input.current_command().id().as_uuid())
                .bind(prepared_instance_safety::claim(input.effect())?)
                .bind(input.configuration().namespace())
                .bind(input.configuration().host())
                .bind(
                    serde_json::to_value(input.configuration().platform())
                        .map_err(|_| DeploymentError::Serialization)?,
                )
                .fetch_one(&mut **tx)
                .await
                .map_err(|error| transaction::storage(&error))?;
        if !qualified {
            return Err(DeploymentError::ReconciliationRequired);
        }
    }
    let native = decode::selected(&context, input.intent(), input.effect().resource.as_str())?;
    let original = decode::closure(native, input)?;
    // The unchanged shared check compares every represented original/tag/Pending
    // pin and retains ALL actual native row bytes for subsequent protected calls.
    let native = origin::checked(native.clone(), input.target(), &original)?;
    Ok((original, native))
}

async fn originals(
    tx: &mut Transaction<'_, Postgres>,
    input: &PreparedInstanceTerminalPreparationInput,
    configuration: &InstanceExecutionConfiguration,
) -> Result<(), DeploymentError> {
    let actual: bool = sqlx::query_scalar(
        "SELECT count(*)=1 AND COALESCE(bool_and(
          child.instance_id=$3 AND child.project_id=header.project_id
          AND create_effect.deployment_id=child.deployment_id AND create_effect.resource_name=child.resource_name
          AND create_effect.project_id=child.project_id AND create_effect.action='create'
          AND effect.deployment_id=child.deployment_id AND effect.resource_name=child.resource_name
          AND effect.project_id=child.project_id AND effect.action IN ('detach','delete')
          AND create_effect.claim_json=$6 AND effect.claim_json=$7
          AND convert_from(child.canonical_import,'UTF8')::jsonb=$8
          AND header.intent_hash=$9 AND activation.config_bytes=$10
          AND current_command.deployment_id=child.deployment_id AND current_command.project_id=child.project_id
          AND current_command.operation='remove' AND current_command.actor_id=$12 AND current_command.idempotency_id=$13),false)
         FROM recipe_prepared_instance_imports child
         JOIN recipe_deployment_execution_preparations header ON header.deployment_id=child.deployment_id
         JOIN recipe_effect_attempts create_effect ON create_effect.id=$4
         JOIN recipe_effect_attempts effect ON effect.id=$5
         JOIN agent_instance_execution_activations activation ON activation.instance_id=child.instance_id
         JOIN recipe_deployment_commands current_command ON current_command.id=$11
         WHERE child.deployment_id=$1 AND child.resource_name=$2")
        .bind(input.intent().id().as_uuid()).bind(input.effect().resource.as_str())
        .bind(input.target().prepared().instance_id().as_uuid())
        .bind(input.target().original().provenance.attempt_id.as_uuid())
        .bind(input.effect().provenance.attempt_id.as_uuid())
        .bind(prepared_instance_safety::claim(input.target().original())?)
        .bind(prepared_instance_safety::claim(input.effect())?)
        .bind(serde_json::to_value(input.target().prepared()).map_err(|_| DeploymentError::Serialization)?)
        .bind(input.intent().input_hash().as_bytes().as_slice())
        .bind(wire::configuration_bytes(configuration)?)
        .bind(input.current_command().id().as_uuid()).bind(input.current_command().actor_id().as_uuid())
        .bind(input.current_command().idempotency_id().as_uuid())
        .fetch_one(&mut **tx).await.map_err(|error| transaction::storage(&error))?;
    if !actual {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}
