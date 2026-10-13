use super::InstanceClosurePublicationOrigin;
use crate::{
    DeploymentError, DeploymentOperation, EffectClaim, PlannedResourceIdentity, ResourceAction,
};
use identity_domain::{RequestId, UserId};
use release_domain::{ContentHash, InstanceRemovalId, ReleaseCommandKey};
use uuid::Uuid;

/// Immutable original permanent closure, never the old actor's credentials.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OriginalInstanceClosure {
    /// Explicit qualified publication origin; never a historical credential.
    pub publication_origin: InstanceClosurePublicationOrigin,
    /// Exact original Drain claim that created the permanent request.
    pub drain: EffectClaim,
    /// Actual request derived from the original Drain attempt.
    pub removal_id: InstanceRemovalId,
    /// Original owning release command key.
    pub command_key: ReleaseCommandKey,
    /// Instance version before permanent closure.
    pub expected_version: u64,
    /// Actual recorded closure publisher, comparison data only.
    pub actor: UserId,
    /// Actual recorded closure publication request, comparison data only.
    pub request: RequestId,
    /// Original exact request fingerprint.
    pub input_hash: ContentHash,
}
impl OriginalInstanceClosure {
    /// Compares closure identities to the original actual Drain claim.
    ///
    /// # Errors
    /// Rejects substituted claims, identities, creator or version.
    pub fn validate(&self, claim: &EffectClaim) -> Result<(), DeploymentError> {
        let drain = &self.drain;
        if drain.deployment_id != claim.deployment_id
            || drain.resource != claim.resource
            || drain.identity != claim.identity
            || drain.input_hash != claim.input_hash
        {
            return Err(DeploymentError::IntentMismatch);
        }
        let PlannedResourceIdentity::Instance { id, .. } = drain.identity else {
            return Err(DeploymentError::IntentMismatch);
        };
        let mut bytes = b"hephaestus-prepared-instance-removal-v1\0".to_vec();
        bytes.extend_from_slice(id.as_uuid().as_bytes());
        let expected = Uuid::new_v5(&drain.provenance.attempt_id.as_uuid(), &bytes);
        let key = ReleaseCommandKey::derive(
            "prepared_instance_remove_v1",
            &[
                id.as_uuid().as_bytes(),
                drain.provenance.attempt_id.as_uuid().as_bytes(),
            ],
        );
        if drain.provenance.actor_id != drain.command.actor_id()
            || drain.provenance.request_id.as_uuid().is_nil()
            || drain.generation == 0
            || drain.resource_version == 0
            || drain.action != ResourceAction::Drain
            || drain.command.operation() != DeploymentOperation::Remove
            || self.removal_id.as_uuid() != expected
            || self.command_key != key
            || self.request.as_uuid().is_nil()
            || self.expected_version == 0
            || self.expected_version > i64::MAX.cast_unsigned()
        {
            return Err(DeploymentError::IntentMismatch);
        }
        match &self.publication_origin {
            InstanceClosurePublicationOrigin::OriginalDrain => {
                if self.actor != drain.provenance.actor_id {
                    return Err(DeploymentError::IntentMismatch);
                }
                Ok(())
            }
            InstanceClosurePublicationOrigin::QualifiedFreshRemove(publication) => {
                publication.validate(self)
            }
        }
    }
}
