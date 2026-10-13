#[path = "resume/association.rs"]
mod association;

use super::{PostgresOwnedReadyRetentionRepository, context};
use crate::{effect_context::Context, receipts, repository_error};
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    AdmissionDisposition, DeploymentAdmission, DeploymentError, DeploymentLifecycle, EffectClaim,
    OwnedReadyRemoveResume, RemoveDeployment,
};
use uuid::Uuid;

pub async fn admit(
    adapter: &PostgresOwnedReadyRetentionRepository,
    identity: &AuthenticatedIdentity,
    remove: &RemoveDeployment,
    target: &EffectClaim,
) -> Result<OwnedReadyRemoveResume, DeploymentError> {
    if remove.deployment_id != target.deployment_id {
        return Err(DeploymentError::InputConflict);
    }
    let mut current = context::manager(adapter, identity, remove.command, target).await?;
    context::prepared(&mut current, target).await?;
    let closure = exact_closure(adapter, &mut current, target).await?;
    let hash = remove.input_hash()?;
    let prior = receipts::find(&mut current.tx, remove.command).await?;
    let prior = prior
        .map(|row| receipts::restore(&row, remove.command, &current.snapshot, hash))
        .transpose()?;
    if let Some(progress) = retained(&mut current, closure).await? {
        current.tx.commit().await.map_err(repository_error)?;
        return Ok(OwnedReadyRemoveResume::Retained(progress));
    }
    if let Some(receipt) = prior {
        let original = context::original_input(adapter, &mut current, remove.command).await?;
        association::ensure(&mut current, identity, remove, closure, &receipt, original).await?;
        receipts::attempt(
            &mut current.tx,
            identity,
            &receipt,
            current.snapshot.intent.project_id(),
            AdmissionDisposition::Resume,
        )
        .await?;
        current.tx.commit().await.map_err(repository_error)?;
        return Ok(OwnedReadyRemoveResume::Admitted(Box::new(
            DeploymentAdmission {
                disposition: AdmissionDisposition::Resume,
                snapshot: current.snapshot,
                receipt,
            },
        )));
    }
    if current.snapshot.lifecycle != DeploymentLifecycle::Removing
        || current.snapshot.version != remove.expected_version
    {
        return Err(DeploymentError::StaleClaim);
    }
    // Current manager receives its own real command. No state/version/old closure
    // rewrite, old actor borrowing, or generic expected-version minus one.
    let receipt = receipts::append(
        &mut current.tx,
        identity,
        remove.command,
        &current.snapshot,
        hash,
        current.organization,
    )
    .await?;
    sqlx::query("INSERT INTO recipe_owned_ready_remove_admissions(command_id,closure_id,expected_version,actor_id,request_id,event_id) VALUES($1,$2,$3,$4,$5,$6)")
        .bind(remove.command.id().as_uuid()).bind(closure)
        .bind(i64::try_from(remove.expected_version).map_err(|_|DeploymentError::StaleClaim)?)
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid()).bind(receipt.event.event_id)
        .execute(&mut *current.tx).await.map_err(repository_error)?;
    receipts::attempt(
        &mut current.tx,
        identity,
        &receipt,
        current.snapshot.intent.project_id(),
        AdmissionDisposition::Created,
    )
    .await?;
    current.tx.commit().await.map_err(repository_error)?;
    Ok(OwnedReadyRemoveResume::Admitted(Box::new(
        DeploymentAdmission {
            disposition: AdmissionDisposition::Created,
            snapshot: current.snapshot,
            receipt,
        },
    )))
}
async fn exact_closure(
    adapter: &PostgresOwnedReadyRetentionRepository,
    current: &mut Context<'_>,
    target: &EffectClaim,
) -> Result<Uuid, DeploymentError> {
    let value:Option<Uuid>=sqlx::query_scalar("SELECT id FROM volume_owned_ready_retention_closures WHERE deployment_id=$1 AND resource_name=$2 AND attempt_id=$3 AND vm_provider_namespace=$4 AND vm_host_id=$5")
        .bind(target.deployment_id.as_uuid()).bind(target.resource.as_str()).bind(target.provenance.attempt_id.as_uuid())
        .bind(&adapter.namespace).bind(&adapter.host).fetch_optional(&mut *current.tx).await.map_err(repository_error)?;
    value.ok_or(DeploymentError::ReconciliationRequired)
}
async fn retained(
    current: &mut Context<'_>,
    closure: Uuid,
) -> Result<Option<recipe_application::ResourceProgress>, DeploymentError> {
    let result:Option<sqlx::types::Json<crate::effect_state::State>>=sqlx::query_scalar("SELECT t.after_state FROM recipe_owned_ready_retentions retained JOIN recipe_execution_transitions t ON t.id=retained.transition_id JOIN recipe_deployment_resources resource ON resource.deployment_id=t.deployment_id AND resource.resource_name=t.resource_name WHERE retained.closure_id=$1 AND t.kind='creation_retained_ready' AND recipe_effect_resource_state(resource)=t.after_state AND resource.install_progress='ready' AND resource.removal_progress='retained' AND resource.active_attempt_id IS NULL")
        .bind(closure).fetch_optional(&mut *current.tx).await.map_err(repository_error)?;
    result.map(|row| row.0.progress()).transpose()
}
