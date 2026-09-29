// `Result::map_err` supplies ownership; only the redacted kind is retained.
#[allow(clippy::needless_pass_by_value)]
pub fn io_error(error: std::io::Error) -> ArtifactStoreError {
    ArtifactStoreError::Io(error.kind())
}

/// Safe-import or immutable-store failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ArtifactStoreError {
    /// Canonical store root is unsafe.
    #[error("release artifact store root is invalid")]
    InvalidStoreRoot,
    /// Sealed build output is not an ordinary directory.
    #[error("sealed build output root is invalid")]
    InvalidSourceRoot,
    /// A path is not a normalized release-relative path.
    #[error("release artifact path is invalid")]
    InvalidPath,
    /// Tree includes a symlink or special filesystem object.
    #[error("sealed build output contains an unsafe object")]
    UnsafeObject,
    /// Hard-linked output could alias outside the sealed tree.
    #[error("sealed build output contains a hard link")]
    HardLink,
    /// Set-user-ID, set-group-ID, and sticky artifact modes are forbidden.
    #[error("sealed build output contains an unsupported mode")]
    UnsafeMode,
    /// Source metadata changed while copying.
    #[error("sealed build output changed during import")]
    SourceChanged,
    /// A stable opaque identity already names different canonical bytes.
    #[error("release artifact storage identity conflicts with canonical bytes")]
    ObjectConflict,
    /// A caller's verified-read byte limit was exceeded.
    #[error("release artifact verified-read limit is invalid")]
    ReadLimit,
    /// Artifact count is empty or exceeds its bound.
    #[error("release artifact count is invalid")]
    FileCount,
    /// Aggregate bytes exceed the import ceiling.
    #[error("release artifact size is invalid")]
    TotalSize,
    /// Redacted I/O failure category.
    #[error("release artifact filesystem operation failed: {0:?}")]
    Io(std::io::ErrorKind),
}
