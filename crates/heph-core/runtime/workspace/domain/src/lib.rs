//! Provider-neutral repository workspace and agent-result contracts.

#[path = "workspace_domain/lifecycle.rs"]
mod lifecycle;
#[path = "workspace_domain/metadata.rs"]
mod metadata;
#[path = "workspace_domain/requests.rs"]
mod requests;

pub use lifecycle::{
    ArtifactId, DisabledWorkspaceManager, PreparedRuntimeGitWorkspace, PreparedWorkspace,
    PublishedResult, ResultId, RunWorkspaceManager, RuntimeGitWorkspaceManager, WorkspaceError,
    WorkspaceId,
};
pub use metadata::{
    PendingResultMetadata, ResultArtifactMetadata, ResultMetadata, ResultRepository,
    WorkspaceMetadata, WorkspaceMetadataRepository,
};
pub use requests::{
    RUNTIME_GIT_GUEST_PATH, RUNTIME_GIT_LOOPBACK_PORT, RuntimeGitWorkspaceRequest,
    WorkspaceRepositoryError, WorkspaceRequestMetadata,
};

#[cfg(test)]
mod tests {
    use super::RuntimeGitWorkspaceRequest;
    use uuid::Uuid;

    fn request() -> RuntimeGitWorkspaceRequest {
        let repository_id = Uuid::new_v4();
        RuntimeGitWorkspaceRequest {
            repository_id,
            target_repository_id: repository_id,
            target_ref: String::from("refs/heads/main"),
            target_commit: "a".repeat(40),
            git_operations: vec![String::from("fetch")],
            ref_globs: vec![String::from("refs/heads/main")],
        }
    }

    #[test]
    fn runtime_git_requires_fetch_and_scoped_branch() {
        let mut value = request();
        value.git_operations.clear();
        assert!(value.validate().is_err());
        value = request();
        value.target_ref = String::from("refs/tags/release");
        assert!(value.validate().is_err());
        value = request();
        value.ref_globs = vec![String::from("refs/heads/other")];
        assert!(value.validate().is_err());
    }
}
