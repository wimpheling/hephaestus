use std::collections::BTreeMap;

use capability_domain::CapabilitySlotKey;
use forge_domain::ProjectId;
use recipe_domain::{RemovalPolicy, ResolvedRecipe, ResolvedResource, ValidatedRecipe};
use release_domain::{ContentHash, ParameterName, ParameterValue};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, VolumeId};
use serde::Serialize;
use uuid::Uuid;

use crate::{DeploymentError, DeploymentId, DeploymentKey, ResourceAction};

/// Immutable provenance, independent of removal intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceOwnership {
    /// Creation belongs to this deployment, subject to live permissions.
    Owned,
    /// Reference-only binding that never transfers ownership.
    External,
}

/// Predicted exact resource identities; no provider handles or host paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PlannedResourceIdentity {
    /// General private-volume identity and owned filesystem identity.
    Volume {
        /// Stable volume identity, externally supplied or deployment-derived.
        id: VolumeId,
        /// Predicted filesystem identity only for an owned volume.
        filesystem_uuid: Option<Uuid>,
    },
    /// Instance and first immutable revision identities.
    Instance {
        /// Predicted instance identity.
        id: AgentInstanceId,
        /// Predicted first revision identity.
        revision_id: AgentInstanceRevisionId,
    },
}

/// Immutable validated resource plan; construction never creates a grant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlannedResource {
    identity: PlannedResourceIdentity,
    ownership: ResourceOwnership,
    removal: RemovalPolicy,
    intent: ResolvedResource,
    input_hash: ContentHash,
}

impl PlannedResource {
    /// Returns the predicted or explicitly external resource identity.
    #[must_use]
    pub const fn identity(&self) -> PlannedResourceIdentity {
        self.identity
    }
    /// Returns immutable creation provenance.
    #[must_use]
    pub const fn ownership(&self) -> ResourceOwnership {
        self.ownership
    }
    /// Returns removal intent, which is never permission to delete.
    #[must_use]
    pub const fn removal(&self) -> RemovalPolicy {
        self.removal
    }
    /// Returns the exact resolved resource and released authority ceilings.
    #[must_use]
    pub const fn intent(&self) -> &ResolvedResource {
        &self.intent
    }
    /// Returns the fingerprint of resolved intent and predicted identities.
    #[must_use]
    pub const fn input_hash(&self) -> ContentHash {
        self.input_hash
    }
    /// Returns immutable dependency names.
    #[must_use]
    pub fn dependencies(&self) -> &[CapabilitySlotKey] {
        match &self.intent {
            ResolvedResource::Volume(volume) => &volume.dependencies,
            ResolvedResource::Instance(instance) => &instance.dependencies,
        }
    }
    /// Validates the ownership and removal ceiling for a proposed action.
    ///
    /// Live authorization and attachment safety remain mandatory in the adapter.
    ///
    /// # Errors
    /// Rejects mutation of external resources and deletion against retention policy.
    pub fn validate_action(&self, action: ResourceAction) -> Result<(), DeploymentError> {
        let allowed = match (self.ownership, action) {
            (ResourceOwnership::External, ResourceAction::VerifyExternal)
            | (ResourceOwnership::Owned, ResourceAction::Create | ResourceAction::Detach) => true,
            (ResourceOwnership::Owned, ResourceAction::Drain) => {
                matches!(self.identity, PlannedResourceIdentity::Instance { .. })
            }
            (ResourceOwnership::Owned, ResourceAction::Retain) => {
                self.removal == RemovalPolicy::Retain
            }
            (ResourceOwnership::Owned, ResourceAction::Delete) => {
                self.removal == RemovalPolicy::Delete
            }
            _ => false,
        };
        if allowed {
            Ok(())
        } else {
            Err(DeploymentError::InvalidAction)
        }
    }
}

/// Validated immutable deployment input, without deserialization or public mutation.
///
/// Hydration reparses the declaration, reloads authoritative catalog/external views,
/// resolves again, and compares recorded fingerprints. This type is not authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentIntent {
    id: DeploymentId,
    project_id: ProjectId,
    key: DeploymentKey,
    declaration_toml: String,
    resolved_json: Vec<u8>,
    declaration_hash: ContentHash,
    resolved_hash: ContentHash,
    input_hash: ContentHash,
    resolved: ResolvedRecipe,
    resources: BTreeMap<CapabilitySlotKey, PlannedResource>,
}

impl DeploymentIntent {
    /// Freezes matching parsed and resolved intent with predicted stable identities.
    ///
    /// Supplied resolution must already come from authoritative adapter views.
    /// Required secret slots are rejected by recipe-domain resolution in version one.
    ///
    /// # Errors
    /// Rejects nil projects, mismatched declaration origins, and serialization errors.
    pub fn new(
        id: DeploymentId,
        project_id: ProjectId,
        key: DeploymentKey,
        declaration: &ValidatedRecipe,
        resolved: &ResolvedRecipe,
    ) -> Result<Self, DeploymentError> {
        if project_id.as_uuid().is_nil() {
            return Err(DeploymentError::InvalidIdentifier);
        }
        let manifest = declaration.manifest();
        if declaration.hash() != resolved.declaration_hash()
            || manifest.contract_version != resolved.contract_version()
            || &manifest.recipe_id != resolved.recipe_id()
            || &manifest.recipe_version != resolved.recipe_version()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let declaration_toml = declaration.canonical_toml()?;
        // Stored source must remain within the same parser bounds on hydration.
        let restored = recipe_domain::parse_recipe(declaration_toml.as_bytes())?;
        if restored.hash() != declaration.hash() {
            return Err(DeploymentError::IntentMismatch);
        }
        let resolved_json = resolved.canonical_json()?;
        let resolved_hash = resolved.hash()?;
        let resources = resolved
            .resources()
            .iter()
            .map(|(name, resource)| {
                plan_resource(id, name, resource).map(|plan| (name.clone(), plan))
            })
            .collect::<Result<_, _>>()?;
        let input_bytes = serde_json::to_vec(&(
            "hephaestus-deployment-intent-v1",
            id,
            project_id,
            &key,
            declaration.hash(),
            resolved_hash,
        ))
        .map_err(|_| DeploymentError::Serialization)?;
        Ok(Self {
            id,
            project_id,
            key,
            declaration_toml,
            resolved_json,
            declaration_hash: declaration.hash(),
            resolved_hash,
            input_hash: ContentHash::digest(&input_bytes),
            resolved: resolved.clone(),
            resources,
        })
    }
    /// Returns the deployment identity, stable across request attempts.
    #[must_use]
    pub const fn id(&self) -> DeploymentId {
        self.id
    }
    /// Returns the exact project owner.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }
    /// Returns the validated project-local key.
    #[must_use]
    pub const fn key(&self) -> &DeploymentKey {
        &self.key
    }
    /// Returns canonical declaration source for later reparsing.
    #[must_use]
    pub fn declaration_toml(&self) -> &str {
        &self.declaration_toml
    }
    /// Returns canonical resolved snapshot bytes.
    #[must_use]
    pub fn resolved_json(&self) -> &[u8] {
        &self.resolved_json
    }
    /// Returns immutable declaration identity.
    #[must_use]
    pub const fn declaration_hash(&self) -> ContentHash {
        self.declaration_hash
    }
    /// Returns resolved graph identity, including ordinary inputs and external bindings.
    #[must_use]
    pub const fn resolved_hash(&self) -> ContentHash {
        self.resolved_hash
    }
    /// Returns complete deployment input identity, separate from command identity.
    #[must_use]
    pub const fn input_hash(&self) -> ContentHash {
        self.input_hash
    }
    /// Returns the validated graph including exact recipe identity/version.
    #[must_use]
    pub const fn resolved(&self) -> &ResolvedRecipe {
        &self.resolved
    }
    /// Returns ordinary supplied or defaulted inputs; secret inputs are unsupported.
    #[must_use]
    pub const fn inputs(&self) -> &BTreeMap<ParameterName, ParameterValue> {
        self.resolved.inputs()
    }
    /// Returns planned resources in canonical name order.
    #[must_use]
    pub const fn resources(&self) -> &BTreeMap<CapabilitySlotKey, PlannedResource> {
        &self.resources
    }
    /// Returns dependency order for install, reversed by callers for removal.
    #[must_use]
    pub fn execution_order(&self) -> &[CapabilitySlotKey] {
        self.resolved.execution_order()
    }
    /// Checks a retry against all immutable previously recorded intent.
    ///
    /// # Errors
    /// Returns a conflict if configuration or deployment identity changed.
    pub fn validate_replay(&self, recorded: &Self) -> Result<(), DeploymentError> {
        if self.input_hash == recorded.input_hash {
            Ok(())
        } else {
            Err(DeploymentError::InputConflict)
        }
    }
}

fn resource_uuid(deployment: DeploymentId, name: &CapabilitySlotKey, purpose: &str) -> Uuid {
    let mut bytes = b"hephaestus-recipe-resource-v1\0".to_vec();
    bytes.extend_from_slice(name.as_str().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(purpose.as_bytes());
    Uuid::new_v5(&deployment.as_uuid(), &bytes)
}

fn plan_resource(
    deployment: DeploymentId,
    name: &CapabilitySlotKey,
    intent: &ResolvedResource,
) -> Result<PlannedResource, DeploymentError> {
    let (identity, ownership, removal) = match intent {
        ResolvedResource::Volume(volume) => volume.external_id.map_or_else(
            || {
                (
                    PlannedResourceIdentity::Volume {
                        id: VolumeId::from_uuid(resource_uuid(deployment, name, "volume")),
                        filesystem_uuid: Some(resource_uuid(deployment, name, "filesystem")),
                    },
                    ResourceOwnership::Owned,
                    volume.removal,
                )
            },
            |id| {
                (
                    PlannedResourceIdentity::Volume {
                        id,
                        filesystem_uuid: None,
                    },
                    ResourceOwnership::External,
                    RemovalPolicy::Retain,
                )
            },
        ),
        ResolvedResource::Instance(instance) => (
            PlannedResourceIdentity::Instance {
                id: AgentInstanceId::from_uuid(resource_uuid(deployment, name, "instance")),
                revision_id: AgentInstanceRevisionId::from_uuid(resource_uuid(
                    deployment, name, "revision",
                )),
            },
            ResourceOwnership::Owned,
            instance.removal,
        ),
    };
    let bytes = serde_json::to_vec(&(identity, ownership, removal, intent))
        .map_err(|_| DeploymentError::Serialization)?;
    Ok(PlannedResource {
        identity,
        ownership,
        removal,
        intent: intent.clone(),
        input_hash: ContentHash::digest(&bytes),
    })
}
