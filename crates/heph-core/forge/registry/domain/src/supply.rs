//! Supply-chain evidence and verification contracts.
use crate::{
    ImmutableManifestReference, OciDescriptor, OciMediaType, PlatformDescriptor,
    RegistryValueError, Sha256Digest,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

/// A required supply-chain referrer category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SupplyChainReferrerKind {
    /// Software bill of materials.
    Sbom,
    /// Build provenance or attestation.
    Provenance,
    /// Vulnerability scan result.
    Scan,
    /// Optional signature or approval artifact.
    Signature,
}

/// One OCI 1.1 supply-chain artifact linked to an immutable subject manifest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SupplyChainReferrer {
    kind: SupplyChainReferrerKind,
    subject: Sha256Digest,
    descriptor: OciDescriptor,
    artifact_type: OciMediaType,
}

impl SupplyChainReferrer {
    /// Creates a supply-chain referrer whose subject is checked during verification.
    #[must_use]
    pub const fn new(
        kind: SupplyChainReferrerKind,
        subject: Sha256Digest,
        descriptor: OciDescriptor,
        artifact_type: OciMediaType,
    ) -> Self {
        Self {
            kind,
            subject,
            descriptor,
            artifact_type,
        }
    }

    /// Returns the supply-chain category.
    #[must_use]
    pub const fn kind(&self) -> SupplyChainReferrerKind {
        self.kind
    }

    /// Returns the referrer's declared subject digest.
    #[must_use]
    pub const fn subject(&self) -> &Sha256Digest {
        &self.subject
    }

    /// Returns the verified artifact descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &OciDescriptor {
        &self.descriptor
    }

    /// Returns the OCI artifact type.
    #[must_use]
    pub const fn artifact_type(&self) -> &OciMediaType {
        &self.artifact_type
    }
}

/// Canonically ordered referrer evidence for one subject digest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyChainEvidence {
    subject: Sha256Digest,
    referrers: Vec<SupplyChainReferrer>,
}

impl SupplyChainEvidence {
    /// Validates and canonically orders supply-chain referrers for one subject.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidReferrer`] when a referrer names a
    /// different subject or two referrers claim the same category.
    pub fn new(
        subject: Sha256Digest,
        mut referrers: Vec<SupplyChainReferrer>,
    ) -> Result<Self, RegistryValueError> {
        let mut kinds = BTreeSet::new();
        let valid = referrers
            .iter()
            .all(|referrer| referrer.subject == subject && kinds.insert(referrer.kind));
        if !valid {
            return Err(RegistryValueError::InvalidReferrer);
        }
        referrers.sort_by(|left, right| {
            left.kind
                .cmp(&right.kind)
                .then_with(|| left.descriptor.digest().cmp(right.descriptor.digest()))
        });
        Ok(Self { subject, referrers })
    }

    /// Returns the immutable subject digest.
    #[must_use]
    pub const fn subject(&self) -> &Sha256Digest {
        &self.subject
    }

    /// Returns canonical supply-chain referrers.
    #[must_use]
    pub fn referrers(&self) -> &[SupplyChainReferrer] {
        &self.referrers
    }

    fn has_kind(&self, kind: SupplyChainReferrerKind) -> bool {
        self.referrers.iter().any(|referrer| referrer.kind == kind)
    }
}

/// Required supply-chain evidence for an immutable publication.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyChainPolicy {
    signature_required: bool,
}

impl SupplyChainPolicy {
    /// Requires SBOM, provenance, and scan evidence, but not a signature.
    #[must_use]
    pub const fn without_signature() -> Self {
        Self {
            signature_required: false,
        }
    }

    /// Requires SBOM, provenance, scan, and signature evidence.
    #[must_use]
    pub const fn with_signature() -> Self {
        Self {
            signature_required: true,
        }
    }

    /// Returns whether an approval signature is required.
    #[must_use]
    pub const fn signature_required(self) -> bool {
        self.signature_required
    }

    pub(crate) fn validate(self, evidence: &SupplyChainEvidence) -> Result<(), RegistryValueError> {
        let required = [
            SupplyChainReferrerKind::Sbom,
            SupplyChainReferrerKind::Provenance,
            SupplyChainReferrerKind::Scan,
        ];
        let baseline_present = required.into_iter().all(|kind| evidence.has_kind(kind));
        (baseline_present
            && (!self.signature_required || evidence.has_kind(SupplyChainReferrerKind::Signature)))
        .then_some(())
        .ok_or(RegistryValueError::MissingRequiredReferrer)
    }
}

impl Default for SupplyChainPolicy {
    fn default() -> Self {
        Self::without_signature()
    }
}

/// A bounded immutable policy revision attached to an intent and approval.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PolicyVersion(String);

impl PolicyVersion {
    /// Parses a bounded printable policy revision.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidPolicyVersion`] when the revision
    /// is absent, oversized, or contains control or whitespace characters.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let valid = (1..=128).contains(&value.len())
            && value == value.trim()
            && !value
                .bytes()
                .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace());
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidPolicyVersion)
    }

    /// Returns the policy revision.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PolicyVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for PolicyVersion {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<PolicyVersion> for String {
    fn from(value: PolicyVersion) -> Self {
        value.0
    }
}

/// Immutable remote verification evidence for one publication intent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerifiedPublication {
    manifest: OciDescriptor,
    platforms: Vec<PlatformDescriptor>,
    evidence: SupplyChainEvidence,
}

impl VerifiedPublication {
    /// Validates descriptors, platforms, and referrer subjects read back from Zot.
    ///
    /// # Errors
    ///
    /// Returns a value error when the manifest digest differs from the
    /// immutable reference, platform manifests are incomplete, or referrers
    /// target a different subject.
    pub fn new(
        reference: &ImmutableManifestReference,
        manifest: OciDescriptor,
        platforms: Vec<PlatformDescriptor>,
        evidence: SupplyChainEvidence,
    ) -> Result<Self, RegistryValueError> {
        let platform_digests = platforms
            .iter()
            .map(|platform| platform.descriptor().digest().clone())
            .collect::<BTreeSet<_>>();
        let valid_manifest = manifest.digest() == reference.digest()
            && (manifest.media_type().is_image_manifest()
                || manifest.media_type().is_image_index())
            && !platforms.is_empty()
            && platform_digests.len() == platforms.len();
        let valid = valid_manifest && evidence.subject() == reference.digest();
        valid
            .then_some(Self {
                manifest,
                platforms,
                evidence,
            })
            .ok_or(RegistryValueError::InvalidVerification)
    }

    /// Returns the verified top-level manifest or index descriptor.
    #[must_use]
    pub const fn manifest(&self) -> &OciDescriptor {
        &self.manifest
    }

    /// Returns verified platform-specific manifest descriptors.
    #[must_use]
    pub fn platforms(&self) -> &[PlatformDescriptor] {
        &self.platforms
    }

    /// Returns verified supply-chain evidence.
    #[must_use]
    pub const fn evidence(&self) -> &SupplyChainEvidence {
        &self.evidence
    }
}

/// Durable publication lifecycle controlled by Hephaestus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    /// Intent exists but no publisher currently holds it.
    Pending,
    /// A trusted publisher is attempting the narrow registry publication.
    Publishing,
    /// Zot content and supply-chain evidence have been verified but not approved.
    Verified,
    /// Verified content is committed as approved and can be consumed by digest.
    Approved,
    /// Historical metadata is retained but new use is prohibited.
    Retired,
    /// Previously approved content is absent or inconsistent in Zot.
    Missing,
}

impl PublicationState {
    /// Returns whether this state permits execution by a consumer.
    #[must_use]
    pub const fn executable(self) -> bool {
        matches!(self, Self::Approved)
    }
}
