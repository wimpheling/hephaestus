//! Parent-only released fixtures; requests, Runs, inboxes and placements use producers.

#[path = "parents/instance.rs"]
mod instance;
#[path = "parents/release.rs"]
mod release;

use forge_domain::{CommitSha, GitRef, ProjectId, Repository, RepositoryId};
use sqlx::PgPool;
use time::OffsetDateTime;
use uuid::Uuid;

pub struct Parents {
    pub(super) repository: Repository,
    pub(super) family: Uuid,
    pub(super) release: Uuid,
    pub(super) agent: Uuid,
    pub(super) instance: Uuid,
    pub(super) revision: Uuid,
    pub(super) attachment: Uuid,
}

impl Parents {
    pub(super) fn new() -> Self {
        Self {
            repository: Repository {
                id: RepositoryId::new(),
                project_id: ProjectId::new(),
                name: "ordinary-routing".into(),
                default_branch: GitRef::parse("refs/heads/main").expect("branch"),
                is_public: false,
                agent_runs_enabled: true,
                created_at: OffsetDateTime::now_utc(),
            },
            family: Uuid::new_v4(),
            release: Uuid::new_v4(),
            agent: Uuid::new_v4(),
            instance: Uuid::new_v4(),
            revision: Uuid::new_v4(),
            attachment: Uuid::new_v4(),
        }
    }

    pub(super) async fn seed(&self, pool: &PgPool, commit: &CommitSha) {
        release::seed(pool, self, commit).await;
        instance::seed(pool, self).await;
    }
}
