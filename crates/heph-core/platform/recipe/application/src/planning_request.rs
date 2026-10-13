use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use recipe_domain::{MAX_RECIPE_ITEMS, MAX_VALUE_BYTES, ValidatedRecipe};
use release_domain::{ParameterName, ParameterValue};
use runtime_types::VolumeId;
use uuid::Uuid;

use crate::{DeploymentError, DeploymentId, DeploymentKey};

/// Bounded ordinary request, containing no caller catalog or resolved graph.
///
/// Construction checks syntax and sizes. It grants no project or resource rights.
#[derive(Debug, Clone)]
pub struct PlanningRequest {
    id: DeploymentId,
    project: ProjectId,
    key: DeploymentKey,
    declaration: ValidatedRecipe,
    inputs: BTreeMap<ParameterName, ParameterValue>,
    external: BTreeMap<CapabilitySlotKey, VolumeId>,
}

impl PlanningRequest {
    /// Parses bounded TOML and derives the project-local deployment identity.
    ///
    /// `UUIDv5` uses the checked project UUID as its namespace and
    /// `hephaestus-recipe-deployment-v1\0` followed by exact UTF-8 key bytes as
    /// its name. Recipe changes never allocate a new identity for the same key.
    ///
    /// # Errors
    /// Rejects malformed TOML, nil identities, excessive maps or oversized strings.
    pub fn new(
        project: ProjectId,
        key: DeploymentKey,
        source: &[u8],
        inputs: BTreeMap<ParameterName, ParameterValue>,
        external: BTreeMap<CapabilitySlotKey, VolumeId>,
    ) -> Result<Self, DeploymentError> {
        if project.as_uuid().is_nil()
            || external.values().any(|id| id.as_uuid().is_nil())
            || inputs.len() > MAX_RECIPE_ITEMS
            || external.len() > MAX_RECIPE_ITEMS
            || inputs.values().any(|value| {
                matches!(value, ParameterValue::String(value) if value.len() > MAX_VALUE_BYTES)
            })
        {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        let mut name = b"hephaestus-recipe-deployment-v1\0".to_vec();
        name.extend_from_slice(key.as_str().as_bytes());
        let id = DeploymentId::from_uuid(Uuid::new_v5(&project.as_uuid(), &name))?;
        Ok(Self {
            id,
            project,
            key,
            declaration: recipe_domain::parse_recipe(source)?,
            inputs,
            external,
        })
    }

    /// Returns the stable deployment identity.
    #[must_use]
    pub const fn id(&self) -> DeploymentId {
        self.id
    }
    /// Returns the target project, whose management permission must be checked.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project
    }
    /// Returns the exact project-local deployment key.
    #[must_use]
    pub const fn key(&self) -> &DeploymentKey {
        &self.key
    }
    /// Returns the parsed bounded declaration.
    #[must_use]
    pub const fn declaration(&self) -> &ValidatedRecipe {
        &self.declaration
    }
    /// Returns typed ordinary inputs.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        &self.inputs
    }
    /// Returns symbolic external names and exact resource identities.
    #[must_use]
    pub const fn external(&self) -> &BTreeMap<CapabilitySlotKey, VolumeId> {
        &self.external
    }
}
