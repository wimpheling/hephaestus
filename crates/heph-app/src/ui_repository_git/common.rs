use git_http::{AuthenticatedHumanGitEndpoint, GitHttpService};
use release_service::{
    UiBrowserRepositoryGitAuthorization, UiGenerationHostResolver, UiNamespace, UiPublicPort,
    UiRepositoryGitOperation,
};
use std::{sync::Arc, time::Duration};

use crate::ui_audit::UiAuditRecorder;

pub(super) const MAX_QUERY_BYTES: usize = 256;
pub(super) const MAX_HEADER_BYTES: usize = 32 * 1024;
pub(super) const MAX_DEADLINE: Duration = Duration::from_secs(15 * 60);
pub(super) const GIT_ACTOR_HEADER: &str = "heph-git-actor-id";

/// State for the reserved UI-origin Git routes.
pub struct UiRepositoryGitState {
    pub(super) host_resolver: Arc<dyn UiGenerationHostResolver>,
    pub(super) authority: Arc<dyn UiBrowserRepositoryGitAuthorization>,
    pub(super) git: Arc<GitHttpService>,
    pub(super) namespace: UiNamespace,
    pub(super) public_port: UiPublicPort,
    pub(super) audit: UiAuditRecorder,
    pub(super) deadline: Duration,
}

impl UiRepositoryGitState {
    /// Builds a bounded adapter around the existing live authority seams.
    pub fn new(
        host_resolver: Arc<dyn UiGenerationHostResolver>,
        authority: Arc<dyn UiBrowserRepositoryGitAuthorization>,
        git: Arc<GitHttpService>,
        namespace: UiNamespace,
        public_port: UiPublicPort,
        audit_sink: Arc<dyn release_service::UiRequestAuditSink>,
    ) -> Self {
        Self {
            host_resolver,
            authority,
            git,
            namespace,
            public_port,
            audit: UiAuditRecorder::new(audit_sink),
            deadline: MAX_DEADLINE,
        }
    }
}

pub(super) struct GitRoute {
    pub(super) repository: String,
    pub(super) endpoint: AuthenticatedHumanGitEndpoint,
    pub(super) authority_operation: UiRepositoryGitOperation,
}
