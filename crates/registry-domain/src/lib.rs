//! Forge-owned OCI registry control-plane contracts.
//!
//! Zot remains authoritative for OCI content. This crate models only the
//! durable ownership, immutable identity, verification, and approval decisions
//! Hephaestus must make before content can be exposed or executed.

use builder_catalog_domain::OciImageId;
use forge_domain::ProjectId;
use runtime_types::ReleaseAgentId;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt, str::FromStr};
use uuid::Uuid;

/// A stable identity for one durable registry publication intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PublicationIntentId(Uuid);

impl PublicationIntentId {
    /// Creates a new publication-intent identity.
    #[must_use]
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    /// Creates an identity from its UUID representation.
    #[must_use]
    pub const fn from_uuid(value: Uuid) -> Self {
        Self(value)
    }

    /// Returns the UUID representation.
    #[must_use]
    pub const fn as_uuid(self) -> Uuid {
        self.0
    }
}

impl Default for PublicationIntentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PublicationIntentId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for PublicationIntentId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

/// A bounded stable key for a platform-owned image namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PlatformImageKey(String);

impl PlatformImageKey {
    /// Parses a canonical platform image key.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidPlatformImageKey`] when the key
    /// is not a bounded lowercase identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let valid = (1..=64).contains(&value.len())
            && value.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || ((byte == b'_' || byte == b'-') && index > 0)
            });
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidPlatformImageKey)
    }

    /// Returns the canonical key.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PlatformImageKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for PlatformImageKey {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<PlatformImageKey> for String {
    fn from(value: PlatformImageKey) -> Self {
        value.0
    }
}

/// The durable resource that exclusively owns one registry namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RegistryOwner {
    /// One platform-owned image selected by its stable key.
    PlatformImage {
        /// Stable platform image key.
        image_key: PlatformImageKey,
    },
    /// One project-owned repository OCI image.
    RepositoryOciImage {
        /// Owning project.
        project_id: ProjectId,
        /// Stable image identity.
        image_id: OciImageId,
    },
    /// One project-owned release agent.
    ReleaseAgent {
        /// Owning project.
        project_id: ProjectId,
        /// Stable release-agent identity.
        release_agent_id: ReleaseAgentId,
    },
}

impl RegistryOwner {
    /// Returns this owner's canonical repository path.
    #[must_use]
    pub fn repository_path(&self) -> String {
        match self {
            Self::PlatformImage { image_key } => {
                format!("platform/images/{image_key}")
            }
            Self::RepositoryOciImage {
                project_id,
                image_id,
            } => format!("projects/{project_id}/repository-images/{image_id}"),
            Self::ReleaseAgent {
                project_id,
                release_agent_id,
            } => format!("projects/{project_id}/release-agents/{release_agent_id}"),
        }
    }
}

/// A canonical registry repository path with no mutable human-name component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RegistryNamespace {
    path: String,
    owner: RegistryOwner,
}

impl RegistryNamespace {
    /// Derives the sole canonical path for an owner.
    #[must_use]
    pub fn for_owner(owner: RegistryOwner) -> Self {
        let path = owner.repository_path();
        Self { path, owner }
    }

    /// Parses one supported canonical namespace path.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidNamespace`] when the path is not
    /// one of the supported paths or contains a non-canonical UUID.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let parts = value.split('/').collect::<Vec<_>>();
        let owner = match parts.as_slice() {
            ["platform", "images", image_key] => RegistryOwner::PlatformImage {
                image_key: PlatformImageKey::parse((*image_key).to_owned())?,
            },
            ["projects", project_id, "repository-images", image_id] => {
                RegistryOwner::RepositoryOciImage {
                    project_id: canonical_project_id(project_id)?,
                    image_id: canonical_project_image_id(image_id)?,
                }
            }
            ["projects", project_id, "release-agents", release_agent_id] => {
                RegistryOwner::ReleaseAgent {
                    project_id: canonical_project_id(project_id)?,
                    release_agent_id: canonical_release_agent_id(release_agent_id)?,
                }
            }
            _ => return Err(RegistryValueError::InvalidNamespace),
        };
        let namespace = Self::for_owner(owner);
        (namespace.path == value)
            .then_some(namespace)
            .ok_or(RegistryValueError::InvalidNamespace)
    }

    /// Returns the canonical slash-separated repository path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.path
    }

    /// Returns the durable owner encoded by this namespace.
    #[must_use]
    pub const fn owner(&self) -> &RegistryOwner {
        &self.owner
    }
}

impl fmt::Display for RegistryNamespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.path.fmt(formatter)
    }
}

impl FromStr for RegistryNamespace {
    type Err = RegistryValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.to_owned())
    }
}

/// Stable ownership of one exact namespace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NamespaceClaim {
    namespace: RegistryNamespace,
    owner: RegistryOwner,
}

impl NamespaceClaim {
    /// Creates the exclusive namespace claim for an owner.
    #[must_use]
    pub fn new(owner: RegistryOwner) -> Self {
        let namespace = RegistryNamespace::for_owner(owner.clone());
        Self { namespace, owner }
    }

    /// Returns the claimed namespace.
    #[must_use]
    pub const fn namespace(&self) -> &RegistryNamespace {
        &self.namespace
    }

    /// Returns the owning resource.
    #[must_use]
    pub const fn owner(&self) -> &RegistryOwner {
        &self.owner
    }

    /// Confirms that an exact namespace belongs to this claim.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryOwnershipError::NamespaceMismatch`] for another
    /// project or resource namespace.
    pub fn assert_owns(&self, namespace: &RegistryNamespace) -> Result<(), RegistryOwnershipError> {
        (self.namespace == *namespace)
            .then_some(())
            .ok_or(RegistryOwnershipError::NamespaceMismatch)
    }
}

/// Registry-authority validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryOwnershipError {
    /// The namespace is not the one derived from the durable owner.
    #[error("registry namespace does not belong to this durable owner")]
    NamespaceMismatch,
}

/// A canonical DNS-style registry authority, optionally with a port.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RegistryAuthority(String);

impl RegistryAuthority {
    /// Parses a canonical registry authority.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidAuthority`] when the authority has
    /// a scheme, path, non-canonical casing, or invalid host/port syntax.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let Some((host, port)) = split_authority(&value) else {
            return Err(RegistryValueError::InvalidAuthority);
        };
        let valid_host = !host.is_empty()
            && host.len() <= 253
            && host.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.' || byte == b'-'
            })
            && !host.starts_with('.')
            && !host.ends_with('.')
            && host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
            });
        let valid_port = port.is_none_or(|port| {
            !port.is_empty()
                && port.bytes().all(|byte| byte.is_ascii_digit())
                && port.parse::<u16>().is_ok_and(|port| port != 0)
        });
        (valid_host && valid_port)
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidAuthority)
    }

    /// Returns the canonical authority.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RegistryAuthority {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for RegistryAuthority {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<RegistryAuthority> for String {
    fn from(value: RegistryAuthority) -> Self {
        value.0
    }
}

/// An immutable lowercase SHA-256 OCI digest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Parses a canonical `sha256:<64 lowercase hexadecimal characters>` digest.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidDigest`] for any other algorithm,
    /// length, casing, or character set.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let Some(hex) = value.strip_prefix("sha256:") else {
            return Err(RegistryValueError::InvalidDigest);
        };
        let valid = hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidDigest)
    }

    /// Returns the canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sha256Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl FromStr for Sha256Digest {
    type Err = RegistryValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.to_owned())
    }
}

impl TryFrom<String> for Sha256Digest {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<Sha256Digest> for String {
    fn from(value: Sha256Digest) -> Self {
        value.0
    }
}

/// An immutable authority, canonical namespace, and SHA-256 manifest reference.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ImmutableManifestReference {
    authority: RegistryAuthority,
    namespace: RegistryNamespace,
    digest: Sha256Digest,
    value: String,
}

impl ImmutableManifestReference {
    /// Builds an immutable manifest reference from validated components.
    #[must_use]
    pub fn new(
        authority: RegistryAuthority,
        namespace: RegistryNamespace,
        digest: Sha256Digest,
    ) -> Self {
        let value = format!("{authority}/{namespace}@{digest}");
        Self {
            authority,
            namespace,
            digest,
            value,
        }
    }

    /// Parses a canonical immutable manifest reference.
    ///
    /// # Errors
    ///
    /// Returns a value error if the authority, namespace, or digest is not
    /// canonical, or if the reference is tag-based.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let Some((location, digest)) = value.rsplit_once('@') else {
            return Err(RegistryValueError::InvalidImmutableReference);
        };
        let Some((authority, namespace)) = location.split_once('/') else {
            return Err(RegistryValueError::InvalidImmutableReference);
        };
        if authority.is_empty() || namespace.is_empty() || value.matches('@').count() != 1 {
            return Err(RegistryValueError::InvalidImmutableReference);
        }
        let reference = Self::new(
            RegistryAuthority::parse(authority.to_owned())?,
            RegistryNamespace::parse(namespace.to_owned())?,
            Sha256Digest::parse(digest.to_owned())?,
        );
        (reference.value == value)
            .then_some(reference)
            .ok_or(RegistryValueError::InvalidImmutableReference)
    }

    /// Returns the registry authority.
    #[must_use]
    pub const fn authority(&self) -> &RegistryAuthority {
        &self.authority
    }

    /// Returns the canonical repository namespace.
    #[must_use]
    pub const fn namespace(&self) -> &RegistryNamespace {
        &self.namespace
    }

    /// Returns the immutable manifest digest.
    #[must_use]
    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Returns the full immutable reference.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.value
    }
}

impl fmt::Display for ImmutableManifestReference {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

impl FromStr for ImmutableManifestReference {
    type Err = RegistryValueError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value.to_owned())
    }
}

/// A validated OCI media type.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct OciMediaType(String);

impl OciMediaType {
    /// The OCI image manifest media type.
    pub const IMAGE_MANIFEST: &'static str = "application/vnd.oci.image.manifest.v1+json";
    /// The OCI image index media type.
    pub const IMAGE_INDEX: &'static str = "application/vnd.oci.image.index.v1+json";

    /// Parses an OCI-compatible application media type without parameters.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidMediaType`] for media types that
    /// contain parameters, uppercase text, or unsupported syntax.
    pub fn parse(value: impl Into<String>) -> Result<Self, RegistryValueError> {
        let value = value.into();
        let valid = (1..=255).contains(&value.len())
            && value.starts_with("application/")
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'/' | b'.' | b'+' | b'-')
            })
            && value.bytes().filter(|byte| *byte == b'/').count() == 1;
        valid
            .then_some(Self(value))
            .ok_or(RegistryValueError::InvalidMediaType)
    }

    /// Returns whether this is an OCI image manifest media type.
    #[must_use]
    pub fn is_image_manifest(&self) -> bool {
        self.0 == Self::IMAGE_MANIFEST
    }

    /// Returns whether this is an OCI image-index media type.
    #[must_use]
    pub fn is_image_index(&self) -> bool {
        self.0 == Self::IMAGE_INDEX
    }

    /// Returns the canonical media type.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for OciMediaType {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<String> for OciMediaType {
    type Error = RegistryValueError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<OciMediaType> for String {
    fn from(value: OciMediaType) -> Self {
        value.0
    }
}

/// A remote OCI descriptor verified by the registry control plane.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OciDescriptor {
    digest: Sha256Digest,
    size: u64,
    media_type: OciMediaType,
}

impl OciDescriptor {
    /// Creates a non-empty OCI descriptor.
    ///
    /// # Errors
    ///
    /// Returns [`RegistryValueError::InvalidDescriptorSize`] when the remote
    /// content has no bytes.
    pub fn new(
        digest: Sha256Digest,
        size: u64,
        media_type: OciMediaType,
    ) -> Result<Self, RegistryValueError> {
        (size != 0)
            .then_some(Self {
                digest,
                size,
                media_type,
            })
            .ok_or(RegistryValueError::InvalidDescriptorSize)
    }

    /// Returns the content digest.
    #[must_use]
    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }

    /// Returns the verified content size in bytes.
    #[must_use]
    pub const fn size(&self) -> u64 {
        self.size
    }

    /// Returns the descriptor media type.
    #[must_use]
    pub const fn media_type(&self) -> &OciMediaType {
        &self.media_type
    }
}

/// One platform-specific image manifest referenced by an OCI index.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct PlatformDescriptor {
    descriptor: OciDescriptor,
    operating_system: String,
    architecture: String,
    variant: Option<String>,
}

impl PlatformDescriptor {
    /// Creates a platform manifest descriptor.
    ///
    /// # Errors
    ///
    /// Returns a value error for a non-manifest descriptor or malformed
    /// operating-system, architecture, or variant label.
    pub fn new(
        descriptor: OciDescriptor,
        operating_system: impl Into<String>,
        architecture: impl Into<String>,
        variant: Option<String>,
    ) -> Result<Self, RegistryValueError> {
        let operating_system = operating_system.into();
        let architecture = architecture.into();
        let valid = descriptor.media_type.is_image_manifest()
            && valid_platform_token(&operating_system)
            && valid_platform_token(&architecture)
            && variant.as_deref().is_none_or(valid_platform_token);
        valid
            .then_some(Self {
                descriptor,
                operating_system,
                architecture,
                variant,
            })
            .ok_or(RegistryValueError::InvalidPlatformDescriptor)
    }

    /// Returns the manifest descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &OciDescriptor {
        &self.descriptor
    }

    /// Returns the OCI operating system label.
    #[must_use]
    pub fn operating_system(&self) -> &str {
        &self.operating_system
    }

    /// Returns the OCI architecture label.
    #[must_use]
    pub fn architecture(&self) -> &str {
        &self.architecture
    }

    /// Returns the optional OCI architecture variant.
    #[must_use]
    pub fn variant(&self) -> Option<&str> {
        self.variant.as_deref()
    }
}

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
                .then_with(|| left.descriptor.digest.cmp(&right.descriptor.digest))
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

    fn validate(self, evidence: &SupplyChainEvidence) -> Result<(), RegistryValueError> {
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
            .map(|platform| platform.descriptor.digest.clone())
            .collect::<BTreeSet<_>>();
        let valid_manifest = manifest.digest == reference.digest
            && (manifest.media_type.is_image_manifest() || manifest.media_type.is_image_index())
            && !platforms.is_empty()
            && platform_digests.len() == platforms.len();
        let valid = valid_manifest && evidence.subject == reference.digest;
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

/// Maximum number of manifest or referrer descriptors accepted in one
/// provider-neutral registry inventory snapshot.
///
/// Keeping this bounded makes the operator report safe to run against an
/// unavailable or unexpectedly large registry without retaining OCI bytes.
pub const MAX_REGISTRY_INVENTORY_ENTRIES: usize = 100_000;

/// A provider-neutral, descriptor-only observation from an OCI registry.
///
/// This intentionally contains neither credentials nor OCI content. Inventory
/// collectors for Zot or another distribution implementation can produce this
/// bounded value before the retention report is evaluated.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RegistryInventoryEntry {
    repository_path: RegistryNamespace,
    digest: Sha256Digest,
}

impl RegistryInventoryEntry {
    /// Creates one canonical descriptor inventory entry.
    #[must_use]
    pub const fn new(repository_path: RegistryNamespace, digest: Sha256Digest) -> Self {
        Self {
            repository_path,
            digest,
        }
    }

    /// Returns the observed repository path.
    #[must_use]
    pub const fn repository_path(&self) -> &RegistryNamespace {
        &self.repository_path
    }

    /// Returns the observed descriptor digest.
    #[must_use]
    pub const fn digest(&self) -> &Sha256Digest {
        &self.digest
    }
}

/// Versioned bounded inventory input accepted by the operator report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryInventoryDocument {
    /// Stable document schema revision.
    pub schema_version: u16,
    /// Manifest and referrer descriptors observed by an inventory provider.
    pub entries: Vec<RegistryInventoryEntry>,
}

/// A deduplicated bounded inventory snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryInventory {
    entries: BTreeSet<RegistryInventoryEntry>,
}

impl TryFrom<RegistryInventoryDocument> for RegistryInventory {
    type Error = RegistryRetentionReportError;

    fn try_from(document: RegistryInventoryDocument) -> Result<Self, Self::Error> {
        if document.schema_version != 1 {
            return Err(RegistryRetentionReportError::UnsupportedInventorySchema(
                document.schema_version,
            ));
        }
        if document.entries.len() > MAX_REGISTRY_INVENTORY_ENTRIES {
            return Err(RegistryRetentionReportError::InventoryTooLarge);
        }
        Ok(Self {
            entries: document.entries.into_iter().collect(),
        })
    }
}

impl RegistryInventory {
    /// Returns the bounded, canonical descriptor inventory.
    #[must_use]
    pub const fn entries(&self) -> &BTreeSet<RegistryInventoryEntry> {
        &self.entries
    }
}

/// A registry object that `PostgreSQL` protects from future retention actions.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct RegistryRetentionRoot {
    repository_path: RegistryNamespace,
    digest: Sha256Digest,
    kind: RegistryRetentionRootKind,
}

impl RegistryRetentionRoot {
    const fn new(
        repository_path: RegistryNamespace,
        digest: Sha256Digest,
        kind: RegistryRetentionRootKind,
    ) -> Self {
        Self {
            repository_path,
            digest,
            kind,
        }
    }

    fn inventory_entry(&self) -> RegistryInventoryEntry {
        RegistryInventoryEntry::new(self.repository_path.clone(), self.digest.clone())
    }
}

/// The durable reason an OCI descriptor remains protected.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryRetentionRootKind {
    /// An approved platform catalog image manifest or index.
    ApprovedCatalog,
    /// An approved project repository-image manifest or index.
    ApprovedRepositoryOciImage,
    /// An approved project release-agent manifest or index.
    ApprovedReleaseAgent,
    /// A pending, publishing, or verified publication intent.
    ActiveIntent,
    /// A platform-specific manifest required by an approved or verified index.
    PlatformManifest,
    /// Required OCI supply-chain evidence for an approved or verified intent.
    RequiredEvidence,
}

/// Counts from the durable notification inbox, with no notification payloads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RegistryNotificationBacklog {
    /// Notifications that have not been claimed.
    pub pending: u64,
    /// Notifications currently leased by a reconciler.
    pub claimed: u64,
    /// Claimed notifications whose lease has expired and may be retried.
    pub expired_claims: u64,
    /// Terminally processed notifications.
    pub processed: u64,
    /// Terminally rejected notifications.
    pub rejected: u64,
}

/// Durable registry lifecycle and inbox observability counters.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct RegistryOperationalMetrics {
    /// Pending publication intents.
    pub pending_publications: u64,
    /// Publication intents currently being published.
    pub publishing_publications: u64,
    /// Verified publication intents awaiting approval.
    pub verified_publications: u64,
    /// Approved publication intents.
    pub approved_publications: u64,
    /// Retired historical publication intents.
    pub retired_publications: u64,
    /// Publications marked unavailable by reconciliation.
    pub missing_publications: u64,
    /// Bounded durable inbox state counts.
    pub notification_backlog: RegistryNotificationBacklog,
}

/// PostgreSQL-derived input to the provider-neutral retention evaluator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRetentionSnapshot {
    intents: Vec<PublicationIntent>,
    metrics: RegistryOperationalMetrics,
}

impl RegistryRetentionSnapshot {
    /// Creates one read-only durable snapshot.
    #[must_use]
    pub const fn new(intents: Vec<PublicationIntent>, metrics: RegistryOperationalMetrics) -> Self {
        Self { intents, metrics }
    }
}

/// Stable, non-destructive comparison of durable roots and registry inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RegistryRetentionReport {
    schema_version: u16,
    mode: RegistryRetentionReportMode,
    inventory_entries: usize,
    retention_roots: Vec<RegistryRetentionRoot>,
    missing_from_inventory: Vec<RegistryRetentionRoot>,
    unreferenced_inventory: Vec<RegistryInventoryEntry>,
    observability: RegistryOperationalMetrics,
    schema_scope: RegistryRetentionSchemaScope,
}

/// This report is deliberately observational; it cannot delete Zot content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RegistryRetentionReportMode {
    /// Lists protected and drifting descriptors without a destructive action.
    ReportOnly,
}

/// Documents the durable OCI ownership represented by the current schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
// These independent capability flags make a partially migrated retention
// schema explicit in operator JSON; combining them into states would hide gaps.
#[allow(clippy::struct_excessive_bools)]
pub struct RegistryRetentionSchemaScope {
    /// Platform catalog image publications are represented.
    pub platform_catalog: bool,
    /// Project repository-image publications are represented.
    pub repository_oci_images: bool,
    /// Project release-agent publications are represented.
    pub release_agents: bool,
    /// Separate generic build-image roots are not represented yet.
    pub generic_build_images: bool,
    /// Separate generic release-artifact OCI roots are not represented yet.
    pub generic_release_artifacts: bool,
}

impl RegistryRetentionReport {
    /// Evaluates a bounded external inventory against durable `PostgreSQL` roots.
    ///
    /// The function has no registry client and no mutation capability. It is
    /// therefore suitable for Zot, another OCI provider, or a saved inventory
    /// document supplied to the operator command.
    #[must_use]
    pub fn evaluate(snapshot: RegistryRetentionSnapshot, inventory: RegistryInventory) -> Self {
        let RegistryRetentionSnapshot { intents, metrics } = snapshot;
        let roots = retention_roots(&intents);
        let observed = inventory.entries;
        let missing_from_inventory = roots
            .iter()
            .filter(|root| !observed.contains(&root.inventory_entry()))
            .cloned()
            .collect();
        let protected = roots
            .iter()
            .map(RegistryRetentionRoot::inventory_entry)
            .collect::<BTreeSet<_>>();
        let unreferenced_inventory = observed.difference(&protected).cloned().collect();
        Self {
            schema_version: 1,
            mode: RegistryRetentionReportMode::ReportOnly,
            inventory_entries: observed.len(),
            retention_roots: roots.into_iter().collect(),
            missing_from_inventory,
            unreferenced_inventory,
            observability: metrics,
            schema_scope: RegistryRetentionSchemaScope {
                platform_catalog: true,
                repository_oci_images: true,
                release_agents: true,
                generic_build_images: false,
                generic_release_artifacts: false,
            },
        }
    }
}

fn retention_roots(intents: &[PublicationIntent]) -> BTreeSet<RegistryRetentionRoot> {
    let mut roots = BTreeSet::new();
    for intent in intents {
        let namespace = intent.reference().namespace().clone();
        let primary_kind = match intent.state() {
            PublicationState::Approved => Some(match intent.claim().owner() {
                RegistryOwner::PlatformImage { .. } => RegistryRetentionRootKind::ApprovedCatalog,
                RegistryOwner::RepositoryOciImage { .. } => {
                    RegistryRetentionRootKind::ApprovedRepositoryOciImage
                }
                RegistryOwner::ReleaseAgent { .. } => {
                    RegistryRetentionRootKind::ApprovedReleaseAgent
                }
            }),
            PublicationState::Pending
            | PublicationState::Publishing
            | PublicationState::Verified => Some(RegistryRetentionRootKind::ActiveIntent),
            PublicationState::Retired | PublicationState::Missing => None,
        };
        let Some(primary_kind) = primary_kind else {
            continue;
        };
        roots.insert(RegistryRetentionRoot::new(
            namespace.clone(),
            intent.reference().digest().clone(),
            primary_kind,
        ));
        if let Some(verification) = intent.verification() {
            for platform in verification.platforms() {
                roots.insert(RegistryRetentionRoot::new(
                    namespace.clone(),
                    platform.descriptor().digest().clone(),
                    RegistryRetentionRootKind::PlatformManifest,
                ));
            }
            for referrer in verification.evidence().referrers() {
                roots.insert(RegistryRetentionRoot::new(
                    namespace.clone(),
                    referrer.descriptor().digest().clone(),
                    RegistryRetentionRootKind::RequiredEvidence,
                ));
            }
        }
    }
    roots
}

/// Retention report input validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryRetentionReportError {
    /// The inventory document has an unsupported schema revision.
    #[error("unsupported registry inventory schema version {0}")]
    UnsupportedInventorySchema(u16),
    /// The inventory exceeds its bounded descriptor count.
    #[error("registry inventory exceeds the bounded descriptor count")]
    InventoryTooLarge,
}

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
        if expected_manifest.digest != reference.digest
            || !(expected_manifest.media_type.is_image_manifest()
                || expected_manifest.media_type.is_image_index())
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
        if verification.manifest != self.expected_manifest {
            return Err(PublicationLifecycleError::UnexpectedManifest);
        }
        self.supply_chain_policy
            .validate(&verification.evidence)
            .map_err(PublicationLifecycleError::InvalidValue)
    }
}

/// Publication lifecycle transition failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PublicationLifecycleError {
    /// The requested lifecycle edge is not legal from the current state.
    #[error("registry publication cannot transition from {from:?} to {to:?}")]
    InvalidTransition {
        /// Current durable state.
        from: PublicationState,
        /// Requested next state.
        to: PublicationState,
    },
    /// Remote verification differs from immutable evidence already retained.
    #[error("registry publication verification conflicts with immutable evidence")]
    ConflictingVerification,
    /// The remote manifest differs from the exact descriptor in the intent.
    #[error("registry publication verification returned an unexpected manifest")]
    UnexpectedManifest,
    /// A lifecycle payload violates a registry value invariant.
    #[error("registry publication lifecycle payload is invalid: {0}")]
    InvalidValue(#[source] RegistryValueError),
}

/// Failure when a consumer requests executable registry content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RegistryConsumptionError {
    /// Verification has not yet committed an approval.
    #[error("registry content is not approved")]
    NotApproved,
    /// Previously approved content is missing or inconsistent in Zot.
    #[error("registry content is missing and must fail closed")]
    MissingContent,
    /// The digest is retained only for history.
    #[error("registry content is retired")]
    Retired,
}

/// Registry value validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryValueError {
    /// Platform image key is malformed.
    #[error("platform image key must be a bounded lowercase identifier")]
    InvalidPlatformImageKey,
    /// Repository path is not one of the supported canonical namespace shapes.
    #[error("registry namespace is not canonical or supported")]
    InvalidNamespace,
    /// Registry authority is not a canonical DNS-style host with optional port.
    #[error("registry authority is invalid")]
    InvalidAuthority,
    /// Digest is not canonical lowercase SHA-256 text.
    #[error("registry digest must be a lowercase sha256 digest")]
    InvalidDigest,
    /// Reference is not a canonical authority/path@sha256 reference.
    #[error("registry manifest reference must be immutable and canonical")]
    InvalidImmutableReference,
    /// OCI media type is not a canonical application media type.
    #[error("OCI media type is invalid")]
    InvalidMediaType,
    /// Descriptor content size must be positive.
    #[error("OCI descriptor size must be positive")]
    InvalidDescriptorSize,
    /// Platform-specific manifest metadata is inconsistent or malformed.
    #[error("OCI platform descriptor is invalid")]
    InvalidPlatformDescriptor,
    /// Referrer has another subject or duplicates a required category.
    #[error("OCI supply-chain referrer is invalid")]
    InvalidReferrer,
    /// A required supply-chain referrer is absent.
    #[error("OCI supply-chain evidence is missing a required referrer")]
    MissingRequiredReferrer,
    /// Policy version is empty, oversized, or non-printable.
    #[error("registry policy version is invalid")]
    InvalidPolicyVersion,
    /// Remote descriptor, platform, or subject verification failed.
    #[error("OCI publication verification is invalid")]
    InvalidVerification,
    /// The intent reference is not owned by its namespace claim.
    #[error("registry intent reference does not match its owner")]
    OwnershipMismatch,
    /// The expected descriptor is not the exact immutable intended manifest.
    #[error("registry intent expected manifest is invalid")]
    InvalidExpectedManifest,
}

fn canonical_project_id(value: &str) -> Result<ProjectId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(ProjectId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

fn canonical_project_image_id(value: &str) -> Result<OciImageId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(OciImageId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

fn canonical_release_agent_id(value: &str) -> Result<ReleaseAgentId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(ReleaseAgentId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

fn split_authority(value: &str) -> Option<(&str, Option<&str>)> {
    let mut parts = value.split(':');
    let host = parts.next()?;
    let port = parts.next();
    parts.next().is_none().then_some((host, port))
}

fn valid_platform_token(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: char = 'a';
    const B: char = 'b';
    const C: char = 'c';
    const D: char = 'd';
    const E: char = 'e';

    fn digest(character: char) -> Sha256Digest {
        Sha256Digest::parse(format!("sha256:{}", character.to_string().repeat(64)))
            .expect("test digest")
    }

    fn authority() -> RegistryAuthority {
        RegistryAuthority::parse("registry.example.test:5443").expect("authority")
    }

    fn owner() -> RegistryOwner {
        RegistryOwner::RepositoryOciImage {
            project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
            image_id: OciImageId::from_uuid(Uuid::from_u128(2)),
        }
    }

    fn reference() -> ImmutableManifestReference {
        ImmutableManifestReference::new(
            authority(),
            RegistryNamespace::for_owner(owner()),
            digest(A),
        )
    }

    fn media_type(value: &str) -> OciMediaType {
        OciMediaType::parse(value).expect("media type")
    }

    fn descriptor(character: char, media_type_text: &str) -> OciDescriptor {
        OciDescriptor::new(digest(character), 42, media_type(media_type_text)).expect("descriptor")
    }

    fn verification(reference: &ImmutableManifestReference) -> VerifiedPublication {
        let subject = reference.digest().clone();
        let referrers = vec![
            SupplyChainReferrer::new(
                SupplyChainReferrerKind::Sbom,
                subject.clone(),
                descriptor(B, "application/spdx+json"),
                media_type("application/spdx+json"),
            ),
            SupplyChainReferrer::new(
                SupplyChainReferrerKind::Provenance,
                subject.clone(),
                descriptor(C, "application/vnd.in-toto+json"),
                media_type("application/vnd.in-toto+json"),
            ),
            SupplyChainReferrer::new(
                SupplyChainReferrerKind::Scan,
                subject.clone(),
                descriptor(D, "application/vnd.hephaestus.scan.v1+json"),
                media_type("application/vnd.hephaestus.scan.v1+json"),
            ),
        ];
        VerifiedPublication::new(
            reference,
            descriptor(A, OciMediaType::IMAGE_INDEX),
            vec![
                PlatformDescriptor::new(
                    descriptor(E, OciMediaType::IMAGE_MANIFEST),
                    "linux",
                    "amd64",
                    None,
                )
                .expect("platform"),
            ],
            SupplyChainEvidence::new(subject, referrers).expect("evidence"),
        )
        .expect("verification")
    }

    fn intent() -> PublicationIntent {
        let reference = reference();
        PublicationIntent::new(
            PublicationIntentId::from_uuid(Uuid::from_u128(3)),
            NamespaceClaim::new(owner()),
            reference,
            descriptor(A, OciMediaType::IMAGE_INDEX),
            PolicyVersion::parse("registry/v1").expect("policy version"),
            SupplyChainPolicy::default(),
        )
        .expect("intent")
    }

    #[test]
    fn canonicalizes_all_supported_namespace_shapes() {
        let platform = RegistryNamespace::for_owner(RegistryOwner::PlatformImage {
            image_key: PlatformImageKey::parse("rust-ubuntu").expect("key"),
        });
        assert_eq!(platform.as_str(), "platform/images/rust-ubuntu");
        assert_eq!(RegistryNamespace::parse(platform.to_string()), Ok(platform));

        let repository = RegistryNamespace::for_owner(owner());
        assert_eq!(
            repository.as_str(),
            "projects/00000000-0000-0000-0000-000000000001/repository-images/00000000-0000-0000-0000-000000000002"
        );
        assert_eq!(
            RegistryNamespace::parse(repository.to_string()),
            Ok(repository)
        );

        let release = RegistryNamespace::for_owner(RegistryOwner::ReleaseAgent {
            project_id: ProjectId::from_uuid(Uuid::from_u128(1)),
            release_agent_id: ReleaseAgentId::from_uuid(Uuid::from_u128(4)),
        });
        assert_eq!(
            release.as_str(),
            "projects/00000000-0000-0000-0000-000000000001/release-agents/00000000-0000-0000-0000-000000000004"
        );
        assert_eq!(RegistryNamespace::parse(release.to_string()), Ok(release));
    }

    #[test]
    fn rejects_noncanonical_paths_and_immutable_references() {
        assert!(
            RegistryNamespace::parse("projects/00000000-0000-0000-0000-000000000001/images/x")
                .is_err()
        );
        assert!(RegistryNamespace::parse("platform/images/Rust").is_err());
        assert!(RegistryNamespace::parse("projects/{00000000-0000-0000-0000-000000000001}/repository-images/00000000-0000-0000-0000-000000000002").is_err());
        assert!(RegistryAuthority::parse("https://registry.example.test").is_err());
        assert!(RegistryAuthority::parse("Registry.example.test").is_err());
        assert!(
            ImmutableManifestReference::parse("registry.example.test/platform/images/rust:latest")
                .is_err()
        );
        assert!(
            ImmutableManifestReference::parse(
                "registry.example.test/platform/images/rust@sha512:abc"
            )
            .is_err()
        );
    }

    #[test]
    fn accepts_only_lowercase_sha256_digests() {
        assert!(Sha256Digest::parse(format!("sha256:{}", "a".repeat(64))).is_ok());
        assert!(Sha256Digest::parse(format!("sha256:{}", "A".repeat(64))).is_err());
        assert!(Sha256Digest::parse(format!("sha256:{}", "g".repeat(64))).is_err());
        assert!(Sha256Digest::parse(format!("sha256:{}", "a".repeat(63))).is_err());
        assert!(Sha256Digest::parse(format!("sha512:{}", "a".repeat(64))).is_err());
    }

    #[test]
    fn ownership_is_exact_and_cross_project_is_denied() {
        let claim = NamespaceClaim::new(owner());
        let same_namespace = RegistryNamespace::for_owner(owner());
        assert!(claim.assert_owns(&same_namespace).is_ok());

        let other_namespace = RegistryNamespace::for_owner(RegistryOwner::RepositoryOciImage {
            project_id: ProjectId::from_uuid(Uuid::from_u128(9)),
            image_id: OciImageId::from_uuid(Uuid::from_u128(2)),
        });
        assert_eq!(
            claim.assert_owns(&other_namespace),
            Err(RegistryOwnershipError::NamespaceMismatch)
        );
        assert!(
            PublicationIntent::new(
                PublicationIntentId::new(),
                claim,
                ImmutableManifestReference::new(authority(), other_namespace, digest(A)),
                descriptor(A, OciMediaType::IMAGE_INDEX),
                PolicyVersion::parse("registry/v1").expect("policy"),
                SupplyChainPolicy::default(),
            )
            .is_err()
        );
    }

    #[test]
    fn approvals_are_immutable_and_transition_idempotently() {
        let pending = intent();
        let publishing = pending.begin_publishing().expect("publishing");
        let retried = publishing.retry().expect("retry");
        let publishing = retried.begin_publishing().expect("publishing");
        let verified = verification(publishing.reference());
        let approved = publishing
            .record_verified(verified.clone())
            .expect("verified")
            .approve()
            .expect("approved");
        let repeated = approved
            .clone()
            .record_verified(verified)
            .expect("same verification")
            .approve()
            .expect("same approval");
        assert_eq!(repeated, approved);
        assert_eq!(approved.approved_reference(), Ok(approved.reference()));

        let conflicting = VerifiedPublication::new(
            approved.reference(),
            descriptor(A, OciMediaType::IMAGE_INDEX),
            vec![
                PlatformDescriptor::new(
                    descriptor('f', OciMediaType::IMAGE_MANIFEST),
                    "linux",
                    "arm64",
                    None,
                )
                .expect("platform"),
            ],
            approved
                .verification()
                .expect("verification")
                .evidence()
                .clone(),
        )
        .expect("valid but different verification");
        assert_eq!(
            approved.record_verified(conflicting),
            Err(PublicationLifecycleError::ConflictingVerification)
        );
    }

    #[test]
    fn missing_content_fails_closed_and_requires_exact_reverification() {
        let publishing = intent().begin_publishing().expect("publishing");
        let verified = verification(publishing.reference());
        let approved = publishing
            .record_verified(verified.clone())
            .expect("verified")
            .approve()
            .expect("approved");
        let missing = approved.mark_missing().expect("missing");
        assert_eq!(
            missing.approved_reference(),
            Err(RegistryConsumptionError::MissingContent)
        );
        let restored = missing.restore_verified(&verified).expect("restored");
        assert_eq!(restored.state(), PublicationState::Approved);

        let retired = restored.retire();
        assert_eq!(retired.state(), PublicationState::Retired);
        assert_eq!(retired.clone().retire(), retired);
        assert_eq!(
            retired.approved_reference(),
            Err(RegistryConsumptionError::Retired)
        );
    }

    #[test]
    fn verification_requires_all_policy_evidence_and_matching_subjects() {
        let reference = reference();
        let subject = reference.digest().clone();
        let incomplete = SupplyChainEvidence::new(
            subject.clone(),
            vec![SupplyChainReferrer::new(
                SupplyChainReferrerKind::Sbom,
                subject.clone(),
                descriptor(B, "application/spdx+json"),
                media_type("application/spdx+json"),
            )],
        )
        .expect("well-formed but incomplete evidence");
        let verification = VerifiedPublication::new(
            &reference,
            descriptor(A, OciMediaType::IMAGE_INDEX),
            vec![
                PlatformDescriptor::new(
                    descriptor(E, OciMediaType::IMAGE_MANIFEST),
                    "linux",
                    "amd64",
                    None,
                )
                .expect("platform"),
            ],
            incomplete,
        )
        .expect("structurally verified");
        assert_eq!(
            intent().record_verified(verification),
            Err(PublicationLifecycleError::InvalidValue(
                RegistryValueError::MissingRequiredReferrer
            ))
        );

        let wrong_subject = SupplyChainEvidence::new(
            subject,
            vec![SupplyChainReferrer::new(
                SupplyChainReferrerKind::Sbom,
                digest(B),
                descriptor(C, "application/spdx+json"),
                media_type("application/spdx+json"),
            )],
        );
        assert_eq!(wrong_subject, Err(RegistryValueError::InvalidReferrer));
    }

    #[test]
    fn retention_report_is_bounded_provider_neutral_and_non_destructive() {
        let publishing = intent().begin_publishing().expect("publishing");
        let verified = verification(publishing.reference());
        let approved = publishing
            .record_verified(verified)
            .expect("verified")
            .approve()
            .expect("approved");
        let namespace = approved.reference().namespace().clone();
        let inventory = RegistryInventory::try_from(RegistryInventoryDocument {
            schema_version: 1,
            entries: vec![
                RegistryInventoryEntry::new(namespace.clone(), digest(A)),
                RegistryInventoryEntry::new(namespace, digest('f')),
            ],
        })
        .expect("bounded inventory");
        let metrics = RegistryOperationalMetrics {
            approved_publications: 1,
            notification_backlog: RegistryNotificationBacklog {
                pending: 2,
                expired_claims: 1,
                ..RegistryNotificationBacklog::default()
            },
            ..RegistryOperationalMetrics::default()
        };

        let report = RegistryRetentionReport::evaluate(
            RegistryRetentionSnapshot::new(vec![approved], metrics),
            inventory,
        );
        let serialized = serde_json::to_value(report).expect("serialize report");

        assert_eq!(serialized["schema_version"], 1);
        assert_eq!(serialized["mode"], "report_only");
        assert_eq!(serialized["inventory_entries"], 2);
        assert_eq!(
            serialized["retention_roots"]
                .as_array()
                .expect("retention roots")
                .len(),
            5
        );
        assert_eq!(
            serialized["missing_from_inventory"]
                .as_array()
                .expect("missing roots")
                .len(),
            4
        );
        assert_eq!(
            serialized["unreferenced_inventory"]
                .as_array()
                .expect("unreferenced inventory")
                .len(),
            1
        );
        assert_eq!(serialized["observability"]["approved_publications"], 1);
        assert_eq!(
            serialized["observability"]["notification_backlog"]["pending"],
            2
        );
        assert_eq!(serialized["schema_scope"]["generic_build_images"], false);
    }

    #[test]
    fn retention_inventory_rejects_unknown_or_unbounded_documents() {
        let unsupported = RegistryInventory::try_from(RegistryInventoryDocument {
            schema_version: 2,
            entries: Vec::new(),
        });
        assert_eq!(
            unsupported,
            Err(RegistryRetentionReportError::UnsupportedInventorySchema(2))
        );

        let entry = RegistryInventoryEntry::new(reference().namespace().clone(), digest(A));
        let oversized = RegistryInventory::try_from(RegistryInventoryDocument {
            schema_version: 1,
            entries: vec![entry; MAX_REGISTRY_INVENTORY_ENTRIES + 1],
        });
        assert_eq!(
            oversized,
            Err(RegistryRetentionReportError::InventoryTooLarge)
        );
    }
}
