use recipe_application::{DeploymentError, PlatformPolicyObservation, SourceObservation};
use release_domain::ContentHash;
use sha2::{Digest, Sha256};

use crate::catalog::CatalogSource;
use crate::evidence::CatalogEvidence;

pub fn observe(
    evidence: &CatalogEvidence,
    source: &CatalogSource,
    platform: &PlatformPolicyObservation,
) -> Result<SourceObservation, DeploymentError> {
    if !evidence.published {
        return Err(DeploymentError::Unavailable);
    }
    // Hash the authored JSON. Effective legacy slots never alter these bytes.
    let bytes = serde_json::to_vec(&source.contract).map_err(|_| DeploymentError::Serialization)?;
    let hash: [u8; 32] = Sha256::digest(&bytes).into();
    if source.hash.as_slice() != hash
        || source.publication_mode != "proposal"
        || source
            .contract
            .get("publication_mode")
            .is_some_and(|mode| mode != "proposal")
        || evidence.capability_requirements.iter().any(|requirement| {
            requirement.slot_required()
                || evidence
                    .volume_slots
                    .iter()
                    .any(|slot| slot.slot() == requirement.slot())
        })
    {
        return Err(DeploymentError::InvalidPlanningInput);
    }
    let image = source
        .contract
        .get("image_reference")
        .and_then(serde_json::Value::as_str)
        .ok_or(DeploymentError::InvalidPlanningInput)?;
    let policy = serde_json::from_value(
        source
            .contract
            .get("policy_ceiling")
            .cloned()
            .ok_or(DeploymentError::InvalidPlanningInput)?,
    )
    .map_err(|_| DeploymentError::InvalidPlanningInput)?;
    SourceObservation::new(
        evidence.pin,
        ContentHash::digest(&bytes),
        image,
        policy,
        platform.clone(),
    )
}
