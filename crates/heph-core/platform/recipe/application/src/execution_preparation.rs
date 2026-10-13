//! Frozen execution inputs, separate from the version-one deployment intent.
//!
//! Construction checks consistency, not publication, permissions or admission.
//! The owning adapter must commit this complete record before any effects.

mod codec;
mod imports;
mod mapping;

use std::collections::BTreeMap;

use forge_domain::ProjectId;
use recipe_domain::ResolvedResource;
use release_domain::ContentHash;
use serde::Serialize;

use crate::{
    CommandIdentity, DeploymentError, DeploymentId, DeploymentIntent, DeploymentOperation,
    PlatformPolicyObservation, SourceObservation,
};

pub use imports::{PreparedInstanceImport, PreparedVolumeSelection};

/// Maximum encoded preparation size, checked before decoding untrusted storage.
pub const MAX_EXECUTION_PREPARATION_BYTES: usize = 1024 * 1024;

/// Closed server-selected execution semantics; neither a grant nor activation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeploymentExecutionProfile {
    /// Exact typed named imports; runtime dispatch requires separate support.
    RuntimeNamedV1,
}

/// Complete checked original input for every instance in one deployment.
///
/// This value has no unchecked deserializer. Stored bytes must be reconstructed
/// against the exact historical intent and original Install command. Current
/// authority is independently mandatory before returning replay or executing.
///
/// ```compile_fail
/// let _: recipe_application::DeploymentExecutionPreparation =
///     serde_json::from_str("{}").unwrap();
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeploymentExecutionPreparation {
    version: u32,
    deployment_id: DeploymentId,
    project_id: ProjectId,
    intent_hash: ContentHash,
    command: CommandIdentity,
    profile: DeploymentExecutionProfile,
    platform: PlatformPolicyObservation,
    instances: Vec<PreparedInstanceImport>,
}

impl DeploymentExecutionPreparation {
    /// Freezes every exact import and the global configuration, including zero imports.
    ///
    /// Sources must come from an authoritative catalog. This constructor does
    /// not authenticate a caller, admit Install, or create any mount grants.
    ///
    /// # Errors
    /// Rejects non-Install commands, incomplete/duplicate source coverage,
    /// inconsistent platform observations, or oversized canonical input.
    pub fn new(
        intent: &DeploymentIntent,
        command: CommandIdentity,
        profile: DeploymentExecutionProfile,
        platform: PlatformPolicyObservation,
        sources: &[SourceObservation],
    ) -> Result<Self, DeploymentError> {
        if command.operation() != DeploymentOperation::Install
            || sources.len() > recipe_domain::MAX_RECIPE_ITEMS
        {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        let mut observed = BTreeMap::new();
        for source in sources {
            if source.platform() != &platform
                || observed
                    .insert(mapping::pin_key(source.pin()), source)
                    .is_some()
            {
                return Err(DeploymentError::InvalidPlanningInput);
            }
        }
        let mut used = std::collections::BTreeSet::new();
        let mut instances = Vec::new();
        for (name, resource) in intent.resources() {
            if let ResolvedResource::Instance(instance) = resource.intent() {
                let key = mapping::pin_key(instance.release);
                let source = observed
                    .get(&key)
                    .ok_or(DeploymentError::InvalidPlanningInput)?;
                used.insert(key);
                instances.push(imports::prepare(intent, command, name, resource, source)?);
            }
        }
        if used.len() != observed.len() {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        let prepared = Self {
            version: 1,
            deployment_id: intent.id(),
            project_id: intent.project_id(),
            intent_hash: intent.input_hash(),
            command,
            profile,
            platform,
            instances,
        };
        prepared.canonical_bytes()?;
        Ok(prepared)
    }

    /// Returns the immutable encoding version.
    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }
    /// Returns the exact deployment identity.
    #[must_use]
    pub const fn deployment_id(&self) -> DeploymentId {
        self.deployment_id
    }
    /// Returns the target project.
    #[must_use]
    pub const fn project_id(&self) -> ProjectId {
        self.project_id
    }
    /// Returns the unchanged version-one intent fingerprint.
    #[must_use]
    pub const fn intent_hash(&self) -> ContentHash {
        self.intent_hash
    }
    /// Returns the original actor-scoped admitted Install command.
    #[must_use]
    pub const fn command(&self) -> CommandIdentity {
        self.command
    }
    /// Returns frozen server-selected semantics, without enabling dispatch.
    #[must_use]
    pub const fn profile(&self) -> DeploymentExecutionProfile {
        self.profile
    }
    /// Returns configured policy/version even for a zero-instance graph.
    #[must_use]
    pub const fn platform(&self) -> &PlatformPolicyObservation {
        &self.platform
    }
    /// Returns complete imports sorted by graph resource name.
    #[must_use]
    pub fn instances(&self) -> &[PreparedInstanceImport] {
        &self.instances
    }

    /// Returns bounded canonical bytes; no per-attempt request is included.
    ///
    /// # Errors
    /// Rejects encoding failure or input larger than the fixed preparation bound.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, DeploymentError> {
        let bytes = serde_json::to_vec(self).map_err(|_| DeploymentError::Serialization)?;
        if bytes.len() > MAX_EXECUTION_PREPARATION_BYTES {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        Ok(bytes)
    }

    /// Fingerprints complete preparation bytes, distinct from release import hashing.
    ///
    /// # Errors
    /// Rejects canonical encoding failure or oversized input.
    pub fn input_hash(&self) -> Result<ContentHash, DeploymentError> {
        self.canonical_bytes()
            .map(|bytes| ContentHash::digest(&bytes))
    }

    /// Reconstructs untrusted storage and compares every canonical field and stable ID.
    ///
    /// Historical intent/catalog evidence must be independently hydrated by the
    /// adapter. This method verifies consistency, never current source authority.
    /// The expected hash must come from immutable authoritative storage, never
    /// from the caller submitting these bytes.
    ///
    /// # Errors
    /// Rejects malformed/noncanonical bytes, unknown fields, changed fingerprints
    /// or any field inconsistent with the expected original intent/command.
    pub fn from_canonical_bytes(
        intent: &DeploymentIntent,
        command: CommandIdentity,
        bytes: &[u8],
        expected_hash: ContentHash,
    ) -> Result<Self, DeploymentError> {
        codec::reconstruct(intent, command, bytes, expected_hash)
    }

    /// Holds execution when configured policy/version/profile changed after preparation.
    ///
    /// This comparison must follow fresh live authorization in the adapter.
    /// Historical inspect and cleanup do not require current configuration equality.
    ///
    /// # Errors
    /// Returns an immutable-input conflict for any configured drift.
    pub fn validate_configuration(
        &self,
        profile: DeploymentExecutionProfile,
        platform: &PlatformPolicyObservation,
    ) -> Result<(), DeploymentError> {
        if self.profile == profile && &self.platform == platform {
            Ok(())
        } else {
            Err(DeploymentError::InputConflict)
        }
    }
}

#[cfg(test)]
mod tests;
