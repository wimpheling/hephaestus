use std::collections::BTreeSet;

use authz_domain::{ObjectRef, ObjectType, Permission};
use forge_domain::ProjectId;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use recipe_application::{
    CommandIdentity, DeploymentError, DeploymentId, DeploymentIntent, DeploymentLifecycle,
    DeploymentOperation, DeploymentSnapshot, InstallProgress, PlatformPolicyObservation,
    SourceObservation,
};
use recipe_domain::{ResourceDeclaration, ValidatedRecipe};
use sqlx::{Postgres, Transaction};

use crate::{
    PostgresDeploymentRepository, catalog, persistence, planning_sources, repository_error,
};

pub async fn project(
    repository: &PostgresDeploymentRepository,
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    deployment: DeploymentId,
) -> Result<ProjectId, DeploymentError> {
    let project = persistence::project(tx, deployment).await?;
    repository
        .require(
            tx,
            identity,
            Permission::CanManage,
            ObjectRef::new(ObjectType::Project, project.as_uuid()),
        )
        .await?;
    Ok(project)
}

pub async fn lock(
    tx: &mut Transaction<'_, Postgres>,
    deployment: DeploymentId,
) -> Result<(), DeploymentError> {
    sqlx::query_scalar::<_, uuid::Uuid>("SELECT id FROM recipe_deployments WHERE id=$1 FOR UPDATE")
        .bind(deployment.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(repository_error)?
        .ok_or(DeploymentError::Unavailable)?;
    Ok(())
}

pub async fn original(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DeploymentIntent,
) -> Result<CommandIdentity, DeploymentError> {
    let rows: Vec<super::rows::OriginalRow> = sqlx::query_as(
        "SELECT command.id,command.actor_id,command.idempotency_id,command.input_hash,
           command.first_request_id,command.receipt_lifecycle,command.receipt_version,
           deployment.created_by,deployment.first_request_id AS deployment_request_id
         FROM recipe_deployment_commands command JOIN recipe_deployments deployment
           ON deployment.id=command.deployment_id AND deployment.project_id=command.project_id
         WHERE command.deployment_id=$1 AND command.operation='install' ORDER BY command.id LIMIT 2",
    )
    .bind(intent.id().as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(repository_error)?;
    let [row] = rows.as_slice() else {
        return Err(DeploymentError::IntentMismatch);
    };
    let command = CommandIdentity::from_recorded(
        UserId::from_uuid(row.actor_id),
        RequestId::from_uuid(row.idempotency_id),
        DeploymentOperation::Install,
    )?;
    if command.id().as_uuid() != row.id
        || row.actor_id != row.created_by
        || row.first_request_id != row.deployment_request_id
        || row.receipt_version != 0
        || row.receipt_lifecycle != "installing"
        || row.input_hash.as_slice() != intent.input_hash().as_bytes()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(command)
}

pub async fn before_effects(
    tx: &mut Transaction<'_, Postgres>,
    snapshot: &DeploymentSnapshot,
) -> Result<(), DeploymentError> {
    if snapshot.lifecycle != DeploymentLifecycle::Installing
        || snapshot.version != 0
        || snapshot.resources.values().any(|resource| {
            resource.install != InstallProgress::Pending
                || resource.version != 0
                || resource.active_attempt.is_some()
        })
    {
        return Err(DeploymentError::ReconciliationRequired);
    }
    let effects: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM recipe_effect_attempts WHERE deployment_id=$1)
         OR EXISTS(SELECT 1 FROM recipe_owned_volume_preparations WHERE deployment_id=$1)
         OR EXISTS(SELECT 1 FROM recipe_deployment_resources resource WHERE resource.deployment_id=$1
           AND (resource.execution_generation<>0
             OR (resource.ownership='owned' AND resource.resource_kind='volume'
               AND EXISTS(SELECT 1 FROM agent_instance_state_volumes volume WHERE volume.id=resource.resource_id))
             OR (resource.resource_kind='instance'
               AND EXISTS(SELECT 1 FROM agent_instances instance WHERE instance.id=resource.resource_id))))",
    )
    .bind(snapshot.intent.id().as_uuid())
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    if effects {
        return Err(DeploymentError::ReconciliationRequired);
    }
    Ok(())
}

pub async fn sources(
    tx: &mut Transaction<'_, Postgres>,
    declaration: &ValidatedRecipe,
    intent: &DeploymentIntent,
    platform: &PlatformPolicyObservation,
) -> Result<Vec<SourceObservation>, DeploymentError> {
    external_ready(tx, intent).await?;
    let evidence = catalog::load(tx, declaration, intent).await?;
    catalog::reconstruct(intent, declaration, &evidence)?;
    let mut seen = BTreeSet::new();
    let mut sources = Vec::new();
    for resource in &declaration.manifest().resources {
        let ResourceDeclaration::Instance(instance) = resource else {
            continue;
        };
        if seen.insert((
            instance.release.release_id,
            instance.release.release_agent_id,
        )) {
            let (catalog, source) = catalog::load_release(tx, instance.release).await?;
            sources.push(planning_sources::observe(&catalog, &source, platform)?);
        }
    }
    Ok(sources)
}

async fn external_ready(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DeploymentIntent,
) -> Result<(), DeploymentError> {
    for resource in intent.resolved().resources().values() {
        let recipe_domain::ResolvedResource::Volume(volume) = resource else {
            continue;
        };
        let Some(id) = volume.external_id else {
            continue;
        };
        let ready: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_instance_state_volumes WHERE id=$1 AND project_id=$2
             AND state IN ('ready','attached') AND provisioning_state='ready' AND capacity_bytes=$3)",
        )
        .bind(id.as_uuid())
        .bind(intent.project_id().as_uuid())
        .bind(i64::try_from(volume.capacity_bytes).map_err(|_| DeploymentError::IntentMismatch)?)
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)?;
        if !ready {
            return Err(DeploymentError::Unavailable);
        }
    }
    Ok(())
}

// Status is a current authentication precondition, not a fabricated FGA deny.
// Historical manager inspection does not reuse an original actor status stamp.
pub async fn active(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
) -> Result<(), DeploymentError> {
    let active: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND status='active')")
            .bind(identity.user_id.as_uuid())
            .fetch_one(&mut **tx)
            .await
            .map_err(repository_error)?;
    if !active {
        return Err(DeploymentError::AuthorizationDenied);
    }
    Ok(())
}

/// Permanent removal admission closes Install even if uncertainty changes lifecycle.
pub async fn install_open(
    tx: &mut Transaction<'_, Postgres>,
    deployment: DeploymentId,
) -> Result<(), DeploymentError> {
    let closed: bool = sqlx::query_scalar("SELECT recipe_cleanup_started($1)")
        .bind(deployment.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)?;
    if closed {
        return Err(DeploymentError::InvalidAction);
    }
    Ok(())
}
