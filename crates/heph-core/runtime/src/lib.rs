//! Provider-neutral runtime contracts for VM, volume, and workspace effects.
//!
//! Concrete providers remain in their leaf crates. This facade exposes the
//! contracts needed by composition and orchestration without exposing provider
//! configuration or persistence implementations.
//!
//! Concrete providers are intentionally absent from this API:
//!
//! ```compile_fail
//! use heph_runtime::LibkrunProvider;
//! ```
//!
//! ```compile_fail
//! use heph_runtime::PostgresVolumeMetadataRepository;
//! ```
//!
//! ```compile_fail
//! use heph_runtime::PgWorkspaceMetadataRepository;
//! ```

pub use vm_trait::{
    BoxedPrivateServiceConnection, DiskFormat, GuestCommand, LogStream, NetworkMode, PortForward,
    PortProtocol, PrivateHttpRequest, PrivateHttpResponse, PrivateHttpServiceSpec,
    PrivateMailboxPublication, RUNTIME_AUTHORITY_CREDENTIAL_BYTES, RUNTIME_GIT_CREDENTIAL_BYTES,
    RootFilesystem, RuntimeAuthorityBootstrap, RuntimeGitBridge, StopMode, VmDisk, VmError,
    VmEvent, VmExit, VmId, VmInstance, VmMetric, VmMount, VmProvider, VmResources, VmSpec,
};

pub use volume_trait::{
    INSTANCE_STATE_DISK_ID, Volume, VolumeAttachment, VolumeError, VolumeKind, VolumeLease,
    VolumeMetadataRepository, VolumeState, VolumeStore,
};

pub use workspace_domain::{
    ArtifactId, DisabledWorkspaceManager, PendingResultMetadata, PreparedRuntimeGitWorkspace,
    PreparedWorkspace, PublishedResult, RUNTIME_GIT_GUEST_PATH, RUNTIME_GIT_LOOPBACK_PORT,
    ResultArtifactMetadata, ResultId, ResultMetadata, ResultRepository, RunWorkspaceManager,
    RuntimeGitWorkspaceManager, RuntimeGitWorkspaceRequest, WorkspaceError, WorkspaceId,
    WorkspaceMetadata, WorkspaceMetadataRepository, WorkspaceRepositoryError,
    WorkspaceRequestMetadata,
};
