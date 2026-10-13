//! Exact committed fact identity after the existing whole-original qualifier.
use super::selected;
use crate::{PostgresDeploymentRepository, repository_error};
use capability_domain::CapabilitySlotKey;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentIntent, DeploymentOperation,
    PlannedResourceIdentity, ResourceOwnership,
};
use uuid::Uuid;

pub async fn load(
    repository: &PostgresDeploymentRepository,
    identity: &AuthenticatedIdentity,
    command: CommandIdentity,
    intent: &DeploymentIntent,
    resource: &CapabilitySlotKey,
) -> Result<Uuid, DeploymentError> {
    command.validate(identity, DeploymentOperation::Remove)?;
    let config = repository
        .remove_resume
        .as_ref()
        .ok_or(DeploymentError::InvalidAction)?;
    let plan = intent
        .resources()
        .get(resource)
        .ok_or(DeploymentError::IntentMismatch)?;
    if plan.ownership() != ResourceOwnership::Owned
        || plan.removal() != recipe_domain::RemovalPolicy::Retain
        || !matches!(
            plan.identity(),
            PlannedResourceIdentity::Volume {
                filesystem_uuid: Some(_),
                ..
            }
        )
    {
        return Err(DeploymentError::InvalidAction);
    }
    let mut tx = repository
        .pool
        .begin_with("BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .await
        .map_err(repository_error)?;
    let result = async {
        let (claim, _) = selected::read(&mut tx, identity, command, intent, resource, config).await?;
        // Facts are NOT unique per closure. Only the committed retention row is;
        // read its actual link, never choose an arbitrary fact with matching data.
        let facts: Vec<Uuid> = sqlx::query_scalar(
            "SELECT retained.fact_id FROM recipe_owned_ready_retentions retained
             JOIN volume_owned_ready_retention_closures closure ON closure.id=retained.closure_id
             JOIN recipe_execution_transitions transition ON transition.id=retained.transition_id
               AND transition.kind='creation_retained_ready' AND transition.ready_retention_fact_id=retained.fact_id
             JOIN volume_owned_ready_retention_facts fact ON fact.id=retained.fact_id AND fact.closure_id=closure.id
             JOIN recipe_deployment_resources resource ON resource.deployment_id=closure.deployment_id
               AND resource.resource_name=closure.resource_name AND recipe_effect_resource_state(resource)=transition.after_state
             JOIN recipe_deployments deployment ON deployment.id=resource.deployment_id
             WHERE closure.deployment_id=$1 AND closure.project_id=$2 AND closure.resource_name=$3 AND closure.attempt_id=$4
               AND transition.attempt_id=closure.attempt_id AND transition.deployment_id=closure.deployment_id
               AND transition.project_id=closure.project_id AND transition.resource_name=closure.resource_name
               AND resource.resource_kind='volume' AND resource.ownership='owned' AND resource.removal_policy='retain'
               AND resource.removal_progress='retained' AND resource.install_progress='ready' AND resource.active_attempt_id IS NULL
               AND closure.vm_provider_namespace=$5 AND closure.vm_host_id=$6
               AND deployment.lifecycle IN ('removing','recovery_required') ORDER BY retained.fact_id LIMIT 2",
        ).bind(intent.id().as_uuid()).bind(intent.project_id().as_uuid()).bind(resource.as_str())
            .bind(claim.provenance.attempt_id.as_uuid()).bind(config.namespace()).bind(config.host())
            .fetch_all(&mut *tx).await.map_err(repository_error)?;
        match facts.as_slice() {
            [fact] if !fact.is_nil() => Ok(*fact),
            _ => Err(DeploymentError::ReconciliationRequired),
        }
    }.await;
    let rollback = tx.rollback().await.map_err(repository_error);
    match result {
        Err(primary) => Err(primary),
        Ok(fact) => rollback.map(|()| fact),
    }
}
