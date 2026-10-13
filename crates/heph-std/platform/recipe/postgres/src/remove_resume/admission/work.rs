//! Existing admission writes and immutable replay; no physical or dispatch IO.
use super::super::decoding;
use crate::{persistence, receipts, repository_error, rows::ReceiptRow};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandReceipt, DeploymentError, DeploymentLifecycle, DeploymentSnapshot, RemoveDeployment,
};
use release_domain::ContentHash;
use serde_json::Value;
use sqlx::{Postgres, Transaction};

pub(super) async fn restore_receipt(
    tx: &mut Transaction<'_, Postgres>,
    remove: RemoveDeployment,
    snapshot: &DeploymentSnapshot,
    roots: &decoding::Roots<'_>,
    prior: &ReceiptRow,
    hash: ContentHash,
    current_data: &Value,
) -> Result<(CommandReceipt, decoding::Current), DeploymentError> {
    if !matches!(
        snapshot.lifecycle,
        DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
    ) {
        return Err(DeploymentError::InvalidAction);
    }
    let receipt = receipts::restore(prior, remove.command, snapshot, hash)?;
    let config = roots.config;
    let data:Option<Value>=sqlx::query_scalar("SELECT context_json FROM recipe_remove_resume_admissions WHERE command_id=$1
        AND expected_version=$2 AND vm_provider_namespace=$3 AND vm_host_id=$4 AND platform_json=$5")
        .bind(remove.command.id().as_uuid()).bind(i64::try_from(remove.expected_version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(config.namespace()).bind(config.host()).bind(serde_json::to_value(config.platform()).map_err(|_|DeploymentError::Serialization)?)
        .fetch_optional(&mut **tx).await.map_err(repository_error)?;
    let data = data.ok_or(DeploymentError::ReconciliationRequired)?;
    let originals = decoding::qualify_replay(
        tx,
        roots,
        snapshot,
        remove.command,
        &decoding::Saved {
            version: remove.expected_version,
            lifecycle: &prior.receipt_lifecycle,
            data: &data,
        },
        current_data,
    )
    .await?;
    Ok((receipt, originals))
}

pub(super) async fn new_receipt(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    remove: RemoveDeployment,
    snapshot: &DeploymentSnapshot,
    roots: &decoding::Roots<'_>,
    hash: ContentHash,
    data: &Value,
) -> Result<(CommandReceipt, decoding::Current), DeploymentError> {
    if !matches!(
        snapshot.lifecycle,
        DeploymentLifecycle::Removing | DeploymentLifecycle::RecoveryRequired
    ) {
        return Err(DeploymentError::InvalidAction);
    }
    if snapshot.version != remove.expected_version {
        return Err(DeploymentError::StaleClaim);
    }
    let decoded = decoding::load(tx, data, roots).await?;
    decoding::validate_receipt(snapshot.lifecycle.as_str(), &decoded)?;
    let originals = decoding::current(decoded, snapshot, remove.command)?;
    let organization = persistence::lock_project(tx, snapshot.intent.project_id()).await?;
    let receipt =
        receipts::append(tx, identity, remove.command, snapshot, hash, organization).await?;
    let config = roots.config;
    sqlx::query("INSERT INTO recipe_remove_resume_admissions(command_id,deployment_id,project_id,expected_version,actor_id,request_id,event_id,
        intent_hash,preparation_hash,vm_provider_namespace,vm_host_id,platform_json,context_json)
        SELECT $1,$2,$3,$4,$5,$6,$7,$8,header.input_hash,$9,$10,$11,$12 FROM recipe_deployment_execution_preparations header WHERE header.deployment_id=$2")
        .bind(remove.command.id().as_uuid()).bind(remove.deployment_id.as_uuid()).bind(snapshot.intent.project_id().as_uuid())
        .bind(i64::try_from(remove.expected_version).map_err(|_|DeploymentError::StaleClaim)?).bind(identity.user_id.as_uuid())
        .bind(identity.request_id.as_uuid()).bind(receipt.event.event_id).bind(snapshot.intent.input_hash().as_bytes().as_slice())
        .bind(config.namespace()).bind(config.host()).bind(serde_json::to_value(config.platform()).map_err(|_|DeploymentError::Serialization)?)
        .bind(data).execute(&mut **tx).await.map_err(repository_error)?;
    Ok((receipt, originals))
}
