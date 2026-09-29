use authz_domain::AuthzError;
use release_artifact_store::ArtifactStoreError;
use uuid::Uuid;

/// Immutable metadata returned with artifact contents.
#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub id: Uuid,
    pub release_id: Uuid,
    pub build_id: Uuid,
    pub source_commit: String,
    pub path: String,
    pub kind: String,
    pub mode: u32,
    pub sha256: String,
    pub size_bytes: u64,
    pub media_type: String,
}

/// Bounded UTF-8 artifact preview.
pub struct ArtifactPreview {
    pub artifact: ArtifactMetadata,
    pub utf8_contents: String,
    pub truncated: bool,
}

/// Typed artifact application failure.
#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    /// No authorized artifact row matched the identifier.
    #[error("artifact was not found")]
    NotFound,
    /// A caller-supplied size limit or cursor was invalid.
    #[error("artifact request is invalid")]
    InvalidArgument,
    /// A caller-supplied bound exceeded the service maximum.
    #[error("artifact request exceeds a service bound")]
    ResourceExhausted,
    /// Previewed bytes were not valid UTF-8.
    #[error("artifact preview is not UTF-8")]
    InvalidUtf8,
    /// Persistence failed while evaluating the authorized query.
    #[error("artifact persistence failed")]
    Persistence(#[source] sqlx::Error),
    /// The canonical artifact store rejected the durable object.
    #[error("artifact storage failed")]
    Storage(#[source] ArtifactStoreError),
    /// Reading an already-authorized immutable object failed.
    #[error("artifact read failed")]
    Io(#[source] std::io::Error),
    /// The transport deadline expired while reading the artifact.
    #[error("artifact read deadline expired")]
    DeadlineExceeded,
    /// Durable metadata did not match the canonical object.
    #[error("artifact metadata is inconsistent")]
    InvalidStoredData,
    /// The authorization evaluator could not decide whether the release is readable.
    #[error("artifact authorization failed")]
    Authorization(#[source] AuthzError),
}
