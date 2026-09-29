//! Immutable OCI manifest identity and descriptors.
use crate::{
    RegistryAuthority, RegistryNamespace, RegistryValueError, Sha256Digest,
    errors::valid_platform_token,
};
use serde::{Deserialize, Serialize};
use std::{fmt, str::FromStr};

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
