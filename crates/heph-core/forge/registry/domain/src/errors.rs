//! Registry domain validation and lifecycle errors.
use crate::supply::PublicationState;
use builder_catalog_domain::OciImageId;
use forge_domain::ProjectId;
use runtime_types::ReleaseAgentId;
use uuid::Uuid;

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

pub fn canonical_project_id(value: &str) -> Result<ProjectId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(ProjectId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

pub fn canonical_project_image_id(value: &str) -> Result<OciImageId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(OciImageId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

pub fn canonical_release_agent_id(value: &str) -> Result<ReleaseAgentId, RegistryValueError> {
    let parsed = Uuid::parse_str(value).map_err(|_| RegistryValueError::InvalidNamespace)?;
    (parsed.to_string() == value)
        .then_some(ReleaseAgentId::from_uuid(parsed))
        .ok_or(RegistryValueError::InvalidNamespace)
}

pub fn split_authority(value: &str) -> Option<(&str, Option<&str>)> {
    let mut parts = value.split(':');
    let host = parts.next()?;
    let port = parts.next();
    parts.next().is_none().then_some((host, port))
}

pub fn valid_platform_token(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        })
}
