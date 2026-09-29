/// Catalog value validation failure.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageCatalogValueError {
    /// The stable key is malformed.
    #[error("image key must be a bounded lowercase identifier")]
    InvalidKey,
    /// The reference is not digest-pinned.
    #[error("OCI image reference must be digest-pinned with a lowercase SHA-256 digest")]
    UnpinnedImage,
    /// The digest is malformed.
    #[error("OCI digest must be a lowercase sha256 digest")]
    InvalidOciDigest,
    /// The display name is malformed.
    #[error("image display name is invalid")]
    InvalidDisplayName,
    /// Toolchain metadata is malformed.
    #[error("image toolchain metadata is invalid")]
    InvalidToolchain,
    /// Architecture metadata is malformed.
    #[error("image architecture metadata is invalid")]
    InvalidArchitecture,
    /// Required provenance or policy metadata is absent.
    #[error("image provenance and platform policy version are required")]
    MissingProvenance,
    /// Stored lifecycle data is unrecognized.
    #[error("image catalog contains an unknown lifecycle value")]
    InvalidStoredValue,
    /// Registry evidence is inconsistent.
    #[error("registry publication metadata is invalid")]
    InvalidRegistryPublication,
}

/// Failure when resolving an OCI image for an execution contract.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ImageSelectionError {
    /// The image is reserved for a platform-owned operation.
    #[error("selected OCI image is reserved for a platform operation")]
    PlatformOperationOnly,
    /// The image is temporarily unavailable.
    #[error("selected OCI image is unavailable")]
    Unavailable,
    /// The image is historical-only.
    #[error("selected OCI image was retired")]
    Retired,
}
