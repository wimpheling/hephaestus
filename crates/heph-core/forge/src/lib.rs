//! Provider-neutral forge contracts and receive-operation DTOs.
//!
//! Repository persistence, bare-Git storage, event publication, personal
//! access-token material, and transport representations remain in their leaf
//! crates. This facade exposes only the stable domain values and service ports
//! needed by application and composition code.
//!
//! Provider and transport implementations are intentionally absent:
//!
//! ```compile_fail
//! use heph_forge::PgForgeRepository;
//! ```
//!
//! ```compile_fail
//! use heph_forge::ForgeNatsOutboxPublisher;
//! ```
//!
//! ```compile_fail
//! use heph_forge::GitStorage;
//! ```
//!
//! ```compile_fail
//! use heph_forge::PersonalAccessToken;
//! ```

pub use forge_domain::{
    AgentConfigRevisionId, CommitSha, GitRef, GitValueError, OrganizationId, Project, ProjectId,
    ReceiveId, RefUpdate, Repository, RepositoryId, RunRequestId, RuntimeReceiveProvenance,
};

pub use forge_service::{
    CreateRepository, ForgeOutboxStore, ForgeRepositoryError, OutboxRecord, ReceiveResult,
    RunRequest,
};
