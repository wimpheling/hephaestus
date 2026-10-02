use std::collections::{BTreeMap, BTreeSet};

use agent_config::SecretSlotDeclaration;
use capability_domain::{CapabilityRequirement, CapabilitySlotKey};
use recipe_application::{DeploymentError, DeploymentIntent};
use recipe_domain::{ResourceDeclaration, ValidatedRecipe};
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use volume_domain::VolumeSlotDeclaration;

use crate::{
    evidence::{AdmissionEvidence, CatalogEvidence, ExternalEvidence},
    repository_error,
};

#[derive(sqlx::FromRow)]
struct CatalogRow {
    published: bool,
    runtime_contract: Value,
    parameter_schema: Value,
    secret_slot_schema: Value,
    requires_state: bool,
    runtime_contract_hash: Vec<u8>,
    publication_mode: String,
}

pub struct CatalogSource {
    pub contract: Value,
    pub hash: Vec<u8>,
    pub publication_mode: String,
}

#[derive(sqlx::FromRow)]
struct RequirementRow {
    id: uuid::Uuid,
    slot_key: String,
    resource_kind: String,
    required_operations: Vec<String>,
    optional_operations: Vec<String>,
    slot_required: bool,
    normalized_hash: Vec<u8>,
}

pub async fn load(
    tx: &mut Transaction<'_, Postgres>,
    declaration: &ValidatedRecipe,
    incoming: &DeploymentIntent,
) -> Result<AdmissionEvidence, DeploymentError> {
    let mut catalog = Vec::new();
    let mut seen = BTreeSet::new();
    for resource in &declaration.manifest().resources {
        let ResourceDeclaration::Instance(instance) = resource else {
            continue;
        };
        if seen.insert(instance.release.release_agent_id) {
            catalog.push(load_release(tx, instance.release).await?.0);
        }
    }
    let mut external = BTreeMap::new();
    for (name, resource) in incoming.resolved().resources() {
        let recipe_domain::ResolvedResource::Volume(volume) = resource else {
            continue;
        };
        let Some(id) = volume.external_id else {
            continue;
        };
        let (project, capacity, state): (uuid::Uuid, i64, String) = sqlx::query_as(
            "SELECT project_id, capacity_bytes, state FROM agent_instance_state_volumes
             WHERE id = $1",
        )
        .bind(id.as_uuid())
        .fetch_optional(&mut **tx)
        .await
        .map_err(repository_error)?
        .ok_or(DeploymentError::Unavailable)?;
        // Typed revision bindings are constrained to this same owning project.
        if project != incoming.project_id().as_uuid()
            || !matches!(state.as_str(), "ready" | "attached")
        {
            return Err(DeploymentError::Unavailable);
        }
        external.insert(
            name.clone(),
            ExternalEvidence {
                id,
                capacity_bytes: u64::try_from(capacity)
                    .map_err(|_| DeploymentError::IntentMismatch)?,
            },
        );
    }
    Ok(AdmissionEvidence { catalog, external })
}

/// Shared catalog validator; historical evidence retains its original shape.
pub async fn load_release(
    tx: &mut Transaction<'_, Postgres>,
    pin: recipe_domain::ReleasePin,
) -> Result<(CatalogEvidence, CatalogSource), DeploymentError> {
    let row: CatalogRow = sqlx::query_as(
        "SELECT release.state = 'published' AS published, agent.runtime_contract,
                    agent.parameter_schema, agent.secret_slot_schema, agent.requires_state,
                    agent.runtime_contract_hash, agent.publication_mode
             FROM release_agents agent JOIN releases release ON release.id = agent.release_id
             WHERE agent.id = $1 AND release.id = $2",
    )
    .bind(pin.release_agent_id.as_uuid())
    .bind(pin.release_id.as_uuid())
    .fetch_optional(&mut **tx)
    .await
    .map_err(repository_error)?
    .ok_or(DeploymentError::Unavailable)?;
    if !row.runtime_contract.is_object()
        || row
            .runtime_contract
            .get("requires_state")
            .is_some_and(|value| value.as_bool() != Some(row.requires_state))
    {
        return Err(DeploymentError::IntentMismatch);
    }
    let authored: Vec<VolumeSlotDeclaration> = row
        .runtime_contract
        .get("volume_slots")
        .map(|value| serde_json::from_value(value.clone()))
        .transpose()
        .map_err(|_| DeploymentError::IntentMismatch)?
        .unwrap_or_default();
    let volume_slots = release_domain::effective_volume_slots(&authored, row.requires_state)
        .map_err(|_| DeploymentError::IntentMismatch)?;
    let secrets: Vec<SecretSlotDeclaration> = serde_json::from_value(row.secret_slot_schema)
        .map_err(|_| DeploymentError::IntentMismatch)?;
    let parameters = serde_json::from_value(row.parameter_schema)
        .map_err(|_| DeploymentError::IntentMismatch)?;
    let evidence = CatalogEvidence {
        pin,
        published: row.published,
        parameters,
        volume_slots,
        required_secret_slot_count: secrets.iter().filter(|slot| slot.required).count(),
        capability_requirements: requirements(tx, pin.release_agent_id.as_uuid()).await?,
    };
    Ok((
        evidence,
        CatalogSource {
            contract: row.runtime_contract,
            hash: row.runtime_contract_hash,
            publication_mode: row.publication_mode,
        },
    ))
}

async fn requirements(
    tx: &mut Transaction<'_, Postgres>,
    agent: uuid::Uuid,
) -> Result<Vec<CapabilityRequirement>, DeploymentError> {
    let rows: Vec<RequirementRow> = sqlx::query_as(
        "SELECT id, slot_key, resource_kind, required_operations, optional_operations,
                slot_required, normalized_hash FROM release_capability_requirements
         WHERE release_agent_id = $1 ORDER BY slot_key, id LIMIT 65",
    )
    .bind(agent)
    .fetch_all(&mut **tx)
    .await
    .map_err(repository_error)?;
    if rows.len() > recipe_domain::MAX_RECIPE_ITEMS {
        return Err(DeploymentError::IntentMismatch);
    }
    rows.into_iter()
        .map(|row| {
            // Domain deserialization reapplies normalized operation and slot validation.
            let requirement: CapabilityRequirement = serde_json::from_value(json!({
                "id": row.id, "slot": row.slot_key, "resource_kind": row.resource_kind,
                "required_operations": row.required_operations,
                "optional_operations": row.optional_operations, "slot_required": row.slot_required,
            }))
            .map_err(|_| DeploymentError::IntentMismatch)?;
            if requirement.normalized_hash().as_bytes() != row.normalized_hash.as_slice() {
                return Err(DeploymentError::IntentMismatch);
            }
            Ok(requirement)
        })
        .collect()
}

pub fn reconstruct(
    incoming: &DeploymentIntent,
    declaration: &ValidatedRecipe,
    evidence: &AdmissionEvidence,
) -> Result<DeploymentIntent, DeploymentError> {
    let catalog = evidence
        .catalog
        .iter()
        .map(CatalogEvidence::view)
        .collect::<Vec<_>>();
    let external: BTreeMap<CapabilitySlotKey, _> = evidence
        .external
        .iter()
        .map(|(name, value)| (name.clone(), value.view()))
        .collect();
    let resolved = declaration.resolve(incoming.inputs(), &external, &catalog)?;
    let authoritative = DeploymentIntent::new(
        incoming.id(),
        incoming.project_id(),
        incoming.key().clone(),
        declaration,
        &resolved,
    )?;
    if &authoritative != incoming {
        return Err(DeploymentError::IntentMismatch);
    }
    Ok(authoritative)
}
