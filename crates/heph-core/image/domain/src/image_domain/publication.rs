use super::{catalog::OciImage, errors::ImageCatalogValueError, identifiers::OciImageReference};
use serde::{Deserialize, Serialize};

/// Durable OCI registry publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryPublicationState {
    /// A publisher has not started the immutable publication.
    Pending,
    /// A trusted publisher owns the current attempt.
    Publishing,
    /// Required evidence has been verified but not approved.
    Verified,
    /// The immutable digest is approved for consumption.
    Approved,
    /// Previously approved content is absent or inconsistent.
    Missing,
    /// The publication is retained only for historical inspection.
    Retired,
}

/// Consumer-visible availability projected from publication state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryAvailabilityState {
    /// The publication is approved and present.
    Available,
    /// The publication is not usable for new work.
    Unavailable,
    /// The publication is historical-only.
    Retired,
}

/// Verification state for one supply-chain evidence kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryEvidenceState {
    /// Verification has not produced evidence yet.
    Pending,
    /// Evidence was verified for the subject digest.
    Verified,
    /// The bound policy does not require this evidence kind.
    NotRequired,
}

/// One immutable OCI evidence reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEvidence {
    /// Durable verification status.
    pub state: RegistryEvidenceState,
    /// Verified immutable evidence reference, when present.
    pub immutable_reference: Option<OciImageReference>,
}

impl RegistryEvidence {
    /// Returns evidence awaiting verification.
    #[must_use]
    pub const fn pending() -> Self {
        Self {
            state: RegistryEvidenceState::Pending,
            immutable_reference: None,
        }
    }
    /// Returns verified evidence.
    #[must_use]
    pub const fn verified(reference: OciImageReference) -> Self {
        Self {
            state: RegistryEvidenceState::Verified,
            immutable_reference: Some(reference),
        }
    }
    /// Returns a policy-excluded evidence kind.
    #[must_use]
    pub const fn not_required() -> Self {
        Self {
            state: RegistryEvidenceState::NotRequired,
            immutable_reference: None,
        }
    }
}

/// Safe projection of a registry publication and its supply-chain evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryPublication {
    /// Durable control-plane state.
    pub state: RegistryPublicationState,
    /// Consumer-visible state derived from the durable state.
    pub availability: RegistryAvailabilityState,
    /// Approved or expected immutable manifest reference.
    pub immutable_reference: Option<OciImageReference>,
    /// Architectures verified from the immutable manifest.
    pub architectures: Vec<String>,
    /// SBOM evidence.
    pub sbom: RegistryEvidence,
    /// Build provenance evidence.
    pub provenance: RegistryEvidence,
    /// Vulnerability scan evidence.
    pub scan: RegistryEvidence,
    /// Optional signature evidence.
    pub signature: RegistryEvidence,
}

impl RegistryPublication {
    /// Validates the safe projection at an adapter boundary.
    ///
    /// # Errors
    ///
    /// Returns [`ImageCatalogValueError::InvalidRegistryPublication`] for an
    /// inconsistent projection.
    pub fn validate(&self) -> Result<(), ImageCatalogValueError> {
        let valid_architectures = !self.architectures.is_empty()
            && self.architectures.len() <= 32
            && self.architectures.iter().all(|architecture| {
                !architecture.is_empty()
                    && architecture.len() <= 64
                    && !architecture
                        .bytes()
                        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace())
            });
        if !valid_architectures {
            return Err(ImageCatalogValueError::InvalidRegistryPublication);
        }
        if [&self.sbom, &self.provenance, &self.scan, &self.signature]
            .into_iter()
            .any(|evidence| {
                matches!(evidence.state, RegistryEvidenceState::Verified)
                    != evidence.immutable_reference.is_some()
            })
        {
            return Err(ImageCatalogValueError::InvalidRegistryPublication);
        }
        match self.state {
            RegistryPublicationState::Approved
                if self.availability == RegistryAvailabilityState::Available
                    && self.immutable_reference.is_some() =>
            {
                Ok(())
            }
            RegistryPublicationState::Retired
                if self.availability == RegistryAvailabilityState::Retired =>
            {
                Ok(())
            }
            RegistryPublicationState::Pending
            | RegistryPublicationState::Publishing
            | RegistryPublicationState::Verified
            | RegistryPublicationState::Missing
                if self.availability == RegistryAvailabilityState::Unavailable =>
            {
                Ok(())
            }
            _ => Err(ImageCatalogValueError::InvalidRegistryPublication),
        }
    }
}

/// Catalog metadata paired with safe registry evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciImagePublication {
    /// Catalog metadata.
    pub image: OciImage,
    /// Registry lifecycle and supply-chain evidence.
    pub registry_publication: RegistryPublication,
}

impl OciImagePublication {
    /// Validates both catalog and registry metadata.
    ///
    /// # Errors
    ///
    /// Returns invalid-data errors for either layer.
    pub fn validate(&self) -> Result<(), ImageCatalogValueError> {
        self.image.validate()?;
        self.registry_publication.validate()
    }
}
