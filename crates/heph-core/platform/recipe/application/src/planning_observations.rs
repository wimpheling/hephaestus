use builder_catalog_domain::OciImageReference;
use recipe_domain::ReleasePin;
use release_domain::{ContentHash, RuntimePolicy};
use serde::Serialize;

use crate::DeploymentError;

/// Server composition's policy observation, not a user policy selection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PlatformPolicyObservation {
    policy: RuntimePolicy,
    version: String,
}

impl PlatformPolicyObservation {
    /// Checks a bounded configured policy and version.
    ///
    /// # Errors
    /// Rejects zero compute allocations or empty, padded, control-containing versions.
    pub fn new(policy: RuntimePolicy, version: String) -> Result<Self, DeploymentError> {
        if policy.vcpus == 0
            || policy.memory_mib == 0
            || version.is_empty()
            || version.len() > 128
            || version.trim() != version
            || version.chars().any(char::is_control)
        {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        Ok(Self { policy, version })
    }
    /// Returns the exact configured ceiling.
    #[must_use]
    pub const fn policy(&self) -> &RuntimePolicy {
        &self.policy
    }
    /// Returns the operator's policy revision.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }
}

/// Checked source facts observed by a trusted catalog, separate from v1 intent.
///
/// These observations are neither grants nor install preview tokens. Install
/// must replan with a fresh authenticated identity and current platform policy.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SourceObservation {
    pin: ReleasePin,
    runtime_contract_hash: ContentHash,
    image: OciImageReference,
    selected_policy: RuntimePolicy,
    platform: PlatformPolicyObservation,
}

impl SourceObservation {
    /// Checks pinned image syntax and selects the exact released ceiling.
    ///
    /// The adapter must first verify publication and the authored contract hash.
    /// Construction itself establishes no source authority or hash provenance.
    ///
    /// # Errors
    /// Rejects nil pins, malformed images, zero allocations or incompatible policy.
    pub fn new(
        pin: ReleasePin,
        runtime_contract_hash: ContentHash,
        image: &str,
        released_policy: RuntimePolicy,
        platform: PlatformPolicyObservation,
    ) -> Result<Self, DeploymentError> {
        if pin.release_id.as_uuid().is_nil()
            || pin.release_agent_id.as_uuid().is_nil()
            || image.len() > recipe_domain::MAX_VALUE_BYTES
            || released_policy.vcpus == 0
            || released_policy.memory_mib == 0
        {
            return Err(DeploymentError::InvalidPlanningInput);
        }
        let image =
            OciImageReference::parse(image).map_err(|_| DeploymentError::InvalidPlanningInput)?;
        RuntimePolicy::resolve(&released_policy, &released_policy, platform.policy())
            .map_err(|_| DeploymentError::IncompatiblePolicy)?;
        Ok(Self {
            pin,
            runtime_contract_hash,
            image,
            selected_policy: released_policy,
            platform,
        })
    }
    /// Returns the exact published source association.
    #[must_use]
    pub const fn pin(&self) -> ReleasePin {
        self.pin
    }
    /// Returns the verified authored JSON fingerprint, before legacy slot lifting.
    #[must_use]
    pub const fn runtime_contract_hash(&self) -> ContentHash {
        self.runtime_contract_hash
    }
    /// Returns the digest-pinned image reference.
    #[must_use]
    pub const fn image(&self) -> &OciImageReference {
        &self.image
    }
    /// Returns the unchanged released policy selected for this profile.
    #[must_use]
    pub const fn selected_policy(&self) -> &RuntimePolicy {
        &self.selected_policy
    }
    /// Returns server platform facts checked at observation time.
    #[must_use]
    pub const fn platform(&self) -> &PlatformPolicyObservation {
        &self.platform
    }
}
