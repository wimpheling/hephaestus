//! Durable publication intents and lifecycle transitions.
use crate::{
    ImmutableManifestReference, NamespaceClaim, OciDescriptor, PolicyVersion, PublicationIntentId,
    PublicationLifecycleError, PublicationState, RegistryConsumptionError, RegistryValueError,
    SupplyChainPolicy, VerifiedPublication,
};
use serde::{Deserialize, Serialize};

/// A durable publication intent and its immutable approval record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicationIntent {
    id: PublicationIntentId,
    claim: NamespaceClaim,
    reference: ImmutableManifestReference,
    expected_manifest: OciDescriptor,
    policy_version: PolicyVersion,
    supply_chain_policy: SupplyChainPolicy,
    state: PublicationState,
    verification: Option<VerifiedPublication>,
}

impl PublicationIntent {
    /// Creates a pending publication intent for one exact immutable reference.
    ///
    /// # Errors
    ///
    /// Returns a value error when the namespace is not owned by the claim or
    /// the expected descriptor does not name the requested immutable digest.
    pub fn new(
        id: PublicationIntentId,
        claim: NamespaceClaim,
        reference: ImmutableManifestReference,
        expected_manifest: OciDescriptor,
        policy_version: PolicyVersion,
        supply_chain_policy: SupplyChainPolicy,
    ) -> Result<Self, RegistryValueError> {
        claim
            .assert_owns(reference.namespace())
            .map_err(|_| RegistryValueError::OwnershipMismatch)?;
        if expected_manifest.digest() != reference.digest()
            || !(expected_manifest.media_type().is_image_manifest()
                || expected_manifest.media_type().is_image_index())
        {
            return Err(RegistryValueError::InvalidExpectedManifest);
        }
        Ok(Self {
            id,
            claim,
            reference,
            expected_manifest,
            policy_version,
            supply_chain_policy,
            state: PublicationState::Pending,
            verification: None,
        })
    }

    /// Returns the durable intent identity.
    #[must_use]
    pub const fn id(&self) -> PublicationIntentId {
        self.id
    }

    /// Returns the namespace ownership claim.
    #[must_use]
    pub const fn claim(&self) -> &NamespaceClaim {
        &self.claim
    }

    /// Returns the sole immutable manifest reference this intent may approve.
    #[must_use]
    pub const fn reference(&self) -> &ImmutableManifestReference {
        &self.reference
    }

    /// Returns the descriptor expected before remote verification begins.
    #[must_use]
    pub const fn expected_manifest(&self) -> &OciDescriptor {
        &self.expected_manifest
    }

    /// Returns the supply-chain policy revision bound to this intent.
    #[must_use]
    pub const fn policy_version(&self) -> &PolicyVersion {
        &self.policy_version
    }

    /// Returns the required supply-chain evidence policy.
    #[must_use]
    pub const fn supply_chain_policy(&self) -> SupplyChainPolicy {
        self.supply_chain_policy
    }

    /// Returns the durable lifecycle state.
    #[must_use]
    pub const fn state(&self) -> PublicationState {
        self.state
    }

    /// Returns immutable remote verification evidence, if recorded.
    #[must_use]
    pub const fn verification(&self) -> Option<&VerifiedPublication> {
        self.verification.as_ref()
    }

    /// Claims this pending intent for publication.
    ///
    /// Repeating the operation while publishing is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`PublicationLifecycleError::InvalidTransition`] after the
    /// intent has progressed beyond the retryable publication states.
    pub fn begin_publishing(mut self) -> Result<Self, PublicationLifecycleError> {
        match self.state {
            PublicationState::Pending => {
                self.state = PublicationState::Publishing;
                Ok(self)
            }
            PublicationState::Publishing => Ok(self),
            state => Err(PublicationLifecycleError::InvalidTransition {
                from: state,
                to: PublicationState::Publishing,
            }),
        }
    }

    /// Returns an interrupted publication to its retryable pending state.
    ///
    /// Repeating the operation while pending is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`PublicationLifecycleError::InvalidTransition`] if remote
    /// verification has already established immutable evidence.
    pub fn retry(mut self) -> Result<Self, PublicationLifecycleError> {
        match self.state {
            PublicationState::Publishing => {
                self.state = PublicationState::Pending;
                Ok(self)
            }
            PublicationState::Pending => Ok(self),
            state => Err(PublicationLifecycleError::InvalidTransition {
                from: state,
                to: PublicationState::Pending,
            }),
        }
    }

    /// Records immutable remote verification evidence.
    ///
    /// Equivalent repeated verification is idempotent. Different evidence is
    /// rejected once the intent has been verified, preventing approval data
    /// from being replaced after the fact.
    ///
    /// # Errors
    ///
    /// Returns a lifecycle error for an illegal state, conflicting evidence,
    /// or evidence that fails the intent's required supply-chain policy.
    pub fn record_verified(
        mut self,
        verification: VerifiedPublication,
    ) -> Result<Self, PublicationLifecycleError> {
        self.validate_verification(&verification)?;
        match self.state {
            PublicationState::Pending | PublicationState::Publishing => {
                self.verification = Some(verification);
                self.state = PublicationState::Verified;
                Ok(self)
            }
            PublicationState::Verified | PublicationState::Approved => {
                if self.verification.as_ref() == Some(&verification) {
                    Ok(self)
                } else {
                    Err(PublicationLifecycleError::ConflictingVerification)
                }
            }
            state => Err(PublicationLifecycleError::InvalidTransition {
                from: state,
                to: PublicationState::Verified,
            }),
        }
    }

    /// Commits the recorded verification as an immutable approval.
    ///
    /// Repeating an approval already committed for the same intent is
    /// idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`PublicationLifecycleError::InvalidTransition`] unless the
    /// intent has first reached `verified` or is already approved.
    pub fn approve(mut self) -> Result<Self, PublicationLifecycleError> {
        match self.state {
            PublicationState::Verified => {
                self.state = PublicationState::Approved;
                Ok(self)
            }
            PublicationState::Approved => Ok(self),
            state => Err(PublicationLifecycleError::InvalidTransition {
                from: state,
                to: PublicationState::Approved,
            }),
        }
    }

    /// Marks previously approved Zot content absent or inconsistent.
    ///
    /// Consumers then fail closed until exact immutable verification is
    /// recorded by [`Self::restore_verified`]. Repeating this observation is
    /// idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`PublicationLifecycleError::InvalidTransition`] unless the
    /// intent was approved or is already marked missing.
    pub fn mark_missing(mut self) -> Result<Self, PublicationLifecycleError> {
        match self.state {
            PublicationState::Approved => {
                self.state = PublicationState::Missing;
                Ok(self)
            }
            PublicationState::Missing => Ok(self),
            state => Err(PublicationLifecycleError::InvalidTransition {
                from: state,
                to: PublicationState::Missing,
            }),
        }
    }

    /// Restores availability only after exact prior verification is observed again.
    ///
    /// # Errors
    ///
    /// Returns [`PublicationLifecycleError::ConflictingVerification`] if the
    /// newly observed Zot graph differs from the immutable approved evidence.
    pub fn restore_verified(
        mut self,
        verification: &VerifiedPublication,
    ) -> Result<Self, PublicationLifecycleError> {
        if self.state != PublicationState::Missing {
            return Err(PublicationLifecycleError::InvalidTransition {
                from: self.state,
                to: PublicationState::Approved,
            });
        }
        self.validate_verification(verification)?;
        if self.verification.as_ref() != Some(verification) {
            return Err(PublicationLifecycleError::ConflictingVerification);
        }
        self.state = PublicationState::Approved;
        Ok(self)
    }

    /// Retires the intent while retaining its immutable historical evidence.
    ///
    /// Repeating retirement is idempotent.
    #[must_use]
    pub const fn retire(mut self) -> Self {
        self.state = PublicationState::Retired;
        self
    }

    /// Returns the exact approved reference only while its content is available.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryConsumptionError::MissingContent`] for a failed
    /// reconciliation observation, so callers cannot execute stale approvals.
    pub const fn approved_reference(
        &self,
    ) -> Result<&ImmutableManifestReference, RegistryConsumptionError> {
        match self.state {
            PublicationState::Approved => Ok(&self.reference),
            PublicationState::Missing => Err(RegistryConsumptionError::MissingContent),
            PublicationState::Retired => Err(RegistryConsumptionError::Retired),
            PublicationState::Pending
            | PublicationState::Publishing
            | PublicationState::Verified => Err(RegistryConsumptionError::NotApproved),
        }
    }

    fn validate_verification(
        &self,
        verification: &VerifiedPublication,
    ) -> Result<(), PublicationLifecycleError> {
        if verification.manifest() != &self.expected_manifest {
            return Err(PublicationLifecycleError::UnexpectedManifest);
        }
        self.supply_chain_policy
            .validate(verification.evidence())
            .map_err(PublicationLifecycleError::InvalidValue)
    }
}
