use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use recipe_application::{
    DeploymentAttemptId, DeploymentError, DeploymentId, DeploymentIntent, DeploymentKey,
    DeploymentSnapshot, PlannedResource, PlannedResourceIdentity, ResourceOwnership,
    ResourceProgress,
};
use release_domain::{ContentHash, ParameterName, ParameterValue};
use sqlx::{Postgres, Transaction};

use crate::{
    evidence::{AdmissionEvidence, CatalogEvidence},
    repository_error,
    rows::{DeploymentRow, ResourceRow},
};

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    id: DeploymentId,
) -> Result<DeploymentSnapshot, DeploymentError> {
    let row: DeploymentRow = sqlx::query_as(
        "SELECT deployment.id, deployment.project_id, deployment.deployment_key,
                deployment.recipe_id, deployment.recipe_version, definition.canonical_toml,
                deployment.declaration_hash, deployment.resolved_json, deployment.resolved_hash,
                deployment.input_hash, deployment.ordinary_inputs, deployment.admission_evidence,
                deployment.resource_count, deployment.lifecycle, deployment.version
         FROM recipe_deployments deployment JOIN recipe_definitions definition
           USING (project_id, recipe_id, recipe_version) WHERE deployment.id = $1",
    )
    .bind(id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?
    .ok_or(DeploymentError::Unavailable)?;
    let lifecycle = row.lifecycle.parse()?;
    let version = unsigned(row.version)?;
    let resource_count =
        usize::try_from(row.resource_count).map_err(|_| DeploymentError::IntentMismatch)?;
    let intent = restore(row)?;
    let rows: Vec<ResourceRow> = sqlx::query_as(
        "SELECT resource_name, resource_kind, resource_id, revision_id, filesystem_uuid,
                ownership, removal_policy, plan_json, input_hash, install_progress,
                removal_progress, version, active_attempt_id, diagnostic
         FROM recipe_deployment_resources WHERE deployment_id = $1 ORDER BY resource_name",
    )
    .bind(id.as_uuid())
    .fetch_all(&mut **tx)
    .await
    .map_err(repository_error)?;
    if rows.len() != resource_count || rows.len() != intent.resources().len() {
        return Err(DeploymentError::IntentMismatch);
    }
    let mut resources = BTreeMap::new();
    for row in rows {
        let name = CapabilitySlotKey::parse(row.resource_name.clone())
            .map_err(|_| DeploymentError::IntentMismatch)?;
        let planned = intent
            .resources()
            .get(&name)
            .ok_or(DeploymentError::IntentMismatch)?;
        validate_resource(&row, planned)?;
        resources.insert(
            name,
            ResourceProgress {
                install: row.install_progress.parse()?,
                removal: row.removal_progress.parse()?,
                version: unsigned(row.version)?,
                active_attempt: row
                    .active_attempt_id
                    .map(DeploymentAttemptId::from_uuid)
                    .transpose()?,
                diagnostic: row.diagnostic.map(|value| value.parse()).transpose()?,
            },
        );
    }
    Ok(DeploymentSnapshot {
        intent,
        lifecycle,
        version,
        resources,
    })
}

fn restore(row: DeploymentRow) -> Result<DeploymentIntent, DeploymentError> {
    let declaration = recipe_domain::parse_recipe(row.canonical_toml.as_bytes())?;
    if declaration.manifest().recipe_id.as_str() != row.recipe_id
        || declaration.manifest().recipe_version.as_str() != row.recipe_version
        || declaration.hash().as_bytes() != row.declaration_hash.as_slice()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let evidence: AdmissionEvidence = serde_json::from_value(row.admission_evidence)
        .map_err(|_| DeploymentError::IntentMismatch)?;
    let supplied: BTreeMap<ParameterName, ParameterValue> =
        serde_json::from_value(row.ordinary_inputs).map_err(|_| DeploymentError::IntentMismatch)?;
    let catalog = evidence
        .catalog
        .iter()
        .map(CatalogEvidence::view)
        .collect::<Vec<_>>();
    let external = evidence
        .external
        .iter()
        .map(|(name, value)| (name.clone(), value.view()))
        .collect();
    let resolved = declaration.resolve(&supplied, &external, &catalog)?;
    let intent = DeploymentIntent::new(
        DeploymentId::from_uuid(row.id)?,
        ProjectId::from_uuid(row.project_id),
        DeploymentKey::parse(row.deployment_key)?,
        &declaration,
        &resolved,
    )?;
    if intent.resolved_json() != row.resolved_json
        || intent.resolved_hash().as_bytes() != row.resolved_hash.as_slice()
        || intent.input_hash().as_bytes() != row.input_hash.as_slice()
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(intent)
}

fn validate_resource(row: &ResourceRow, planned: &PlannedResource) -> Result<(), DeploymentError> {
    let (kind, id, revision, filesystem) = identity_fields(planned.identity());
    let ownership = match planned.ownership() {
        ResourceOwnership::Owned => "owned",
        ResourceOwnership::External => "external",
    };
    let removal = match planned.removal() {
        recipe_domain::RemovalPolicy::Retain => "retain",
        recipe_domain::RemovalPolicy::Delete => "delete",
    };
    if row.resource_kind != kind
        || row.resource_id != id
        || row.revision_id != revision
        || row.filesystem_uuid != filesystem
        || row.ownership != ownership
        || row.removal_policy != removal
        || row.input_hash.as_slice() != planned.input_hash().as_bytes()
        || row.plan_json
            != serde_json::to_value(planned).map_err(|_| DeploymentError::Serialization)?
    {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(())
}

pub const fn identity_fields(
    identity: PlannedResourceIdentity,
) -> (
    &'static str,
    uuid::Uuid,
    Option<uuid::Uuid>,
    Option<uuid::Uuid>,
) {
    match identity {
        PlannedResourceIdentity::Volume {
            id,
            filesystem_uuid,
        } => ("volume", id.as_uuid(), None, filesystem_uuid),
        PlannedResourceIdentity::Instance { id, revision_id } => {
            ("instance", id.as_uuid(), Some(revision_id.as_uuid()), None)
        }
    }
}

pub fn digest(bytes: &[u8]) -> Result<ContentHash, DeploymentError> {
    bytes
        .try_into()
        .map(ContentHash::from_digest)
        .map_err(|_| DeploymentError::IntentMismatch)
}

pub fn unsigned(value: i64) -> Result<u64, DeploymentError> {
    u64::try_from(value).map_err(|_| DeploymentError::IntentMismatch)
}
