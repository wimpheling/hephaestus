use crate::{execution_preparation, repository_error};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, EffectClaim, RemoveResumeConfiguration,
};
use sqlx::{Postgres, Transaction};

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Reservations {
    commands: Vec<i64>,
    instances: Vec<[i32; 2]>,
}

pub async fn configured(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DeploymentIntent,
    config: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let stored = execution_preparation::hydration::load(tx, intent)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?;
    stored
        .preparation
        .validate_configuration(config.profile(), config.platform())
}
pub async fn reserve(
    tx: &mut Transaction<'_, Postgres>,
    deployment: recipe_application::DeploymentId,
) -> Result<(), DeploymentError> {
    // Only this freshly owned adapter transaction blocks, before ANY parent lock.
    // The read-only loader authorizes comparison data, never lock/IO authority.
    let value: serde_json::Value = sqlx::query_scalar("SELECT remove_resume_reservations118($1)")
        .bind(deployment.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)?;
    let keys: Reservations =
        serde_json::from_value(value).map_err(|_| DeploymentError::Serialization)?;
    if keys.commands.windows(2).any(|pair| pair[0] >= pair[1])
        || keys.instances.windows(2).any(|pair| pair[0] >= pair[1])
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    for key in keys.commands {
        sqlx::query("SELECT pg_advisory_xact_lock($1::bigint)")
            .bind(key)
            .execute(&mut **tx)
            .await
            .map_err(repository_error)?;
    }
    for [first, second] in keys.instances {
        sqlx::query("SELECT pg_advisory_xact_lock($1::integer,$2::integer)")
            .bind(first)
            .bind(second)
            .execute(&mut **tx)
            .await
            .map_err(repository_error)?;
    }
    // SQL always TRYs the current complete set and repeats fresh permissions.
    sqlx::query("SELECT reserve_remove_resume118($1)")
        .bind(deployment.as_uuid())
        .execute(&mut **tx)
        .await
        .map_err(repository_error)?;
    Ok(())
}
pub async fn selected(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    claim: &EffectClaim,
    config: Option<&RemoveResumeConfiguration>,
) -> Result<bool, DeploymentError> {
    let Some(config) = config else {
        return Ok(false);
    };
    sqlx::query_scalar("SELECT qualified_remove_resume_claim118($1,$2,$3,$4,$5)")
        .bind(command.id().as_uuid())
        .bind(
            serde_json::to_value(crate::execution_rows::ClaimWire::new(claim)?)
                .map_err(|_| DeploymentError::Serialization)?,
        )
        .bind(config.namespace())
        .bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_| DeploymentError::Serialization)?)
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)
}

pub async fn command_configuration(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    config: &RemoveResumeConfiguration,
) -> Result<(), DeploymentError> {
    let valid: bool=sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM recipe_remove_resume_admissions WHERE command_id=$1)
        OR EXISTS(SELECT 1 FROM recipe_remove_resume_admissions a JOIN recipe_deployment_execution_preparations p ON p.deployment_id=a.deployment_id
          WHERE a.command_id=$1 AND a.vm_provider_namespace=$2 AND a.vm_host_id=$3 AND a.platform_json=$4 AND a.preparation_hash=p.input_hash)")
        .bind(command.id().as_uuid()).bind(config.namespace()).bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_|DeploymentError::Serialization)?)
        .fetch_one(&mut **tx).await.map_err(repository_error)?;
    if valid {
        Ok(())
    } else {
        Err(DeploymentError::ReconciliationRequired)
    }
}

/// New continuation receipts retain their original current CAS; ordinary Remove
/// receipts retain the old transition-incremented fingerprint path unchanged.
pub async fn current_input(
    tx: &mut Transaction<'_, Postgres>,
    command: CommandIdentity,
    snapshot: &recipe_application::DeploymentSnapshot,
    config: Option<&RemoveResumeConfiguration>,
) -> Result<Option<release_domain::ContentHash>, DeploymentError> {
    let Some(config) = config else {
        return Ok(None);
    };
    let recorded: Option<(i64,bool,String,serde_json::Value)> = sqlx::query_as("SELECT a.expected_version,
        (a.deployment_id=$2 AND a.project_id=$3 AND a.actor_id=$4 AND a.intent_hash=$5
          AND a.vm_provider_namespace=$6 AND a.vm_host_id=$7 AND a.platform_json=$8
          AND EXISTS(SELECT 1 FROM recipe_deployment_commands c
            JOIN recipe_deployment_execution_preparations p ON p.deployment_id=c.deployment_id
            WHERE c.id=a.command_id AND c.operation='remove' AND c.deployment_id=a.deployment_id
              AND c.project_id=a.project_id AND c.actor_id=a.actor_id AND c.first_request_id=a.request_id
              AND c.event_id=a.event_id AND c.receipt_lifecycle IN ('removing','recovery_required') AND c.receipt_version=a.expected_version
              AND p.project_id=a.project_id AND p.intent_hash=a.intent_hash AND p.profile='runtime_named_v1'
              AND p.input_hash=a.preparation_hash AND p.input_hash=sha256(p.canonical_input)
              AND p.platform_json=a.platform_json)),
        (SELECT c.receipt_lifecycle FROM recipe_deployment_commands c WHERE c.id=a.command_id),a.context_json
        FROM recipe_remove_resume_admissions a WHERE a.command_id=$1")
        .bind(command.id().as_uuid()).bind(snapshot.intent.id().as_uuid())
        .bind(snapshot.intent.project_id().as_uuid()).bind(command.actor_id().as_uuid())
        .bind(snapshot.intent.input_hash().as_bytes().as_slice()).bind(config.namespace()).bind(config.host())
        .bind(serde_json::to_value(config.platform()).map_err(|_|DeploymentError::Serialization)?)
        .fetch_optional(&mut **tx).await.map_err(repository_error)?;
    let Some((expected, valid, lifecycle, data)) = recorded else {
        return Ok(None);
    };
    if !valid || expected <= 0 {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let preparation = execution_preparation::hydration::load(tx, &snapshot.intent)
        .await?
        .ok_or(DeploymentError::ReconciliationRequired)?
        .preparation;
    let roots = super::decoding::Roots {
        intent: &snapshot.intent,
        preparation: &preparation,
        config,
    };
    let current: serde_json::Value = sqlx::query_scalar("SELECT remove_resume_context118($1)")
        .bind(snapshot.intent.id().as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(super::decoding::storage)?;
    // Historical prefix and current State are intentionally separate. Native
    // winner loading never invokes after-publication Pending-only absence.
    super::decoding::qualify_replay(
        tx,
        &roots,
        snapshot,
        command,
        &super::decoding::Saved {
            version: crate::hydration::unsigned(expected)?,
            lifecycle: &lifecycle,
            data: &data,
        },
        &current,
    )
    .await?;
    let input = recipe_application::RemoveDeployment {
        command,
        deployment_id: snapshot.intent.id(),
        expected_version: crate::hydration::unsigned(expected)?,
    }
    .input_hash()?;
    Ok(Some(input))
}
