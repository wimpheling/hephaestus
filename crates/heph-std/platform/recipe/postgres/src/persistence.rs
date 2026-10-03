use forge_domain::ProjectId;
use identity_domain::AuthenticatedIdentity;
use recipe_application::{DeploymentError, DeploymentId, DeploymentIntent, ResourceOwnership};
use recipe_domain::RemovalPolicy;
use sqlx::{Postgres, Transaction};

use crate::{evidence::AdmissionEvidence, hydration::identity_fields, repository_error};

pub fn write_error(error: sqlx::Error) -> DeploymentError {
    if error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .is_some_and(|code| code == "23505")
    {
        DeploymentError::InputConflict
    } else {
        repository_error(error)
    }
}

pub async fn lock_project(
    tx: &mut Transaction<'_, Postgres>,
    project: ProjectId,
) -> Result<uuid::Uuid, DeploymentError> {
    sqlx::query_scalar("SELECT organization_id FROM projects WHERE id = $1 FOR NO KEY UPDATE")
        .bind(project.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(repository_error)?
        .ok_or(DeploymentError::Unavailable)
}

pub async fn project(
    tx: &mut Transaction<'_, Postgres>,
    id: DeploymentId,
) -> Result<ProjectId, DeploymentError> {
    sqlx::query_scalar::<_, Option<uuid::Uuid>>("SELECT recipe_deployment_project($1)")
        .bind(id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .map_err(repository_error)?
        .map(ProjectId::from_uuid)
        .ok_or(DeploymentError::Unavailable)
}

pub async fn existing_deployment(
    tx: &mut Transaction<'_, Postgres>,
    intent: &DeploymentIntent,
) -> Result<Option<DeploymentId>, DeploymentError> {
    let ids: Vec<uuid::Uuid> = sqlx::query_scalar(
        "SELECT id FROM recipe_deployments WHERE id = $1 OR (project_id = $2 AND deployment_key = $3)",
    ).bind(intent.id().as_uuid()).bind(intent.project_id().as_uuid()).bind(intent.key().as_str())
        .fetch_all(&mut **tx).await.map_err(repository_error)?;
    if ids.len() > 1 || ids.first().is_some_and(|id| *id != intent.id().as_uuid()) {
        return Err(DeploymentError::InputConflict);
    }
    ids.first()
        .copied()
        .map(DeploymentId::from_uuid)
        .transpose()
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    identity: &AuthenticatedIdentity,
    intent: &DeploymentIntent,
    evidence: &AdmissionEvidence,
) -> Result<(), DeploymentError> {
    sqlx::query(
        "INSERT INTO recipe_definitions (project_id, recipe_id, recipe_version, contract_version,
            canonical_toml, declaration_hash, created_by, first_request_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT (project_id, recipe_id, recipe_version) DO NOTHING",
    ).bind(intent.project_id().as_uuid()).bind(intent.resolved().recipe_id().as_str())
        .bind(intent.resolved().recipe_version().as_str())
        .bind(i32::try_from(intent.resolved().contract_version()).map_err(|_| DeploymentError::IntentMismatch)?)
        .bind(intent.declaration_toml()).bind(intent.declaration_hash().as_bytes().as_slice())
        .bind(identity.user_id.as_uuid()).bind(identity.request_id.as_uuid())
        .execute(&mut **tx).await.map_err(write_error)?;
    let (hash, canonical): (Vec<u8>, String) = sqlx::query_as(
        "SELECT declaration_hash, canonical_toml FROM recipe_definitions
         WHERE project_id = $1 AND recipe_id = $2 AND recipe_version = $3",
    )
    .bind(intent.project_id().as_uuid())
    .bind(intent.resolved().recipe_id().as_str())
    .bind(intent.resolved().recipe_version().as_str())
    .fetch_one(&mut **tx)
    .await
    .map_err(repository_error)?;
    if hash.as_slice() != intent.declaration_hash().as_bytes()
        || canonical != intent.declaration_toml()
    {
        return Err(DeploymentError::InputConflict);
    }
    sqlx::query(
        "INSERT INTO recipe_deployments (id, project_id, deployment_key, recipe_id, recipe_version,
            declaration_hash, resolved_json, resolved_hash, input_hash, ordinary_inputs,
            admission_evidence, resource_count, created_by, first_request_id)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
    )
    .bind(intent.id().as_uuid())
    .bind(intent.project_id().as_uuid())
    .bind(intent.key().as_str())
    .bind(intent.resolved().recipe_id().as_str())
    .bind(intent.resolved().recipe_version().as_str())
    .bind(intent.declaration_hash().as_bytes().as_slice())
    .bind(intent.resolved_json())
    .bind(intent.resolved_hash().as_bytes().as_slice())
    .bind(intent.input_hash().as_bytes().as_slice())
    .bind(serde_json::to_value(intent.inputs()).map_err(|_| DeploymentError::Serialization)?)
    .bind(serde_json::to_value(evidence).map_err(|_| DeploymentError::Serialization)?)
    .bind(i32::try_from(intent.resources().len()).map_err(|_| DeploymentError::IntentMismatch)?)
    .bind(identity.user_id.as_uuid())
    .bind(identity.request_id.as_uuid())
    .execute(&mut **tx)
    .await
    .map_err(write_error)?;
    for (name, resource) in intent.resources() {
        let (kind, id, revision, filesystem) = identity_fields(resource.identity());
        let ownership = match resource.ownership() {
            ResourceOwnership::Owned => "owned",
            ResourceOwnership::External => "external",
        };
        let removal = match resource.removal() {
            RemovalPolicy::Retain => "retain",
            RemovalPolicy::Delete => "delete",
        };
        sqlx::query(
            "INSERT INTO recipe_deployment_resources (deployment_id, project_id, resource_name,
                resource_kind, resource_id, revision_id, filesystem_uuid, ownership, removal_policy,
                plan_json, input_hash, removal_progress) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)",
        ).bind(intent.id().as_uuid()).bind(intent.project_id().as_uuid()).bind(name.as_str())
            .bind(kind).bind(id).bind(revision).bind(filesystem).bind(ownership).bind(removal)
            .bind(serde_json::to_value(resource).map_err(|_| DeploymentError::Serialization)?)
            .bind(resource.input_hash().as_bytes().as_slice())
            .bind(if ownership == "external" { "retained" } else { "pending" })
            .execute(&mut **tx).await.map_err(write_error)?;
    }
    Ok(())
}
