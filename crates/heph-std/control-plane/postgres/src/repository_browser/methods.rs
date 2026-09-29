use forge_domain::RepositoryId;
use forge_service::GitStorage;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    Branch, BrowserApplication, BrowserError, Commit, CommitDetail, CommitDetailRequest, TreeEntry,
    commit::{is_ancestor, parse_commit_detail, select_parent},
    diff_ops::{changed_files, populate_hunks},
    diff_parse::validate_object_id,
    git::{
        git, git_owned, parse_branches, parse_commits, parse_tree, resolve_branch, validate_path,
    },
};

impl BrowserApplication {
    pub const fn new(pool: PgPool, storage: Arc<GitStorage>) -> Self {
        Self { pool, storage }
    }

    pub async fn branches(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<Vec<Branch>, BrowserError> {
        let repository = self.repository(identity, id).await?;
        let output = git(
            &repository,
            &[
                "for-each-ref",
                "--sort=refname",
                "--format=%(refname)%00%(objectname)%00%(committerdate:unix)%00%(subject)",
                "refs/heads/",
            ],
            1_048_576,
        )
        .await?;
        parse_branches(&output)
    }

    pub async fn commits(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        branch: &str,
        skip: usize,
        limit: usize,
    ) -> Result<(Branch, Vec<Commit>), BrowserError> {
        let repository = self.repository(identity, id).await?;
        let selected = resolve_branch(&repository, branch).await?;
        let skip_arg = format!("--skip={skip}");
        let count_arg = format!("--max-count={limit}");
        let output = git(
            &repository,
            &[
                "log",
                "-z",
                &skip_arg,
                &count_arg,
                "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s",
                &selected.commit,
            ],
            2_097_152,
        )
        .await?;
        Ok((selected, parse_commits(&output)?))
    }

    pub async fn tree(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        branch: &str,
    ) -> Result<(Branch, Vec<TreeEntry>), BrowserError> {
        let repository = self.repository(identity, id).await?;
        let selected = resolve_branch(&repository, branch).await?;
        let output = git(
            &repository,
            &["ls-tree", "-r", "-z", "-l", "--full-tree", &selected.commit],
            4_194_304,
        )
        .await?;
        Ok((selected, parse_tree(&output)?))
    }

    pub async fn blob(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        branch: &str,
        path: &str,
        maximum: usize,
    ) -> Result<(Branch, TreeEntry, Vec<u8>), BrowserError> {
        validate_path(path)?;
        let repository = self.repository(identity, id).await?;
        let selected = resolve_branch(&repository, branch).await?;
        let output = git(
            &repository,
            &["ls-tree", "-z", "-l", &selected.commit, "--", path],
            16_384,
        )
        .await?;
        let entry = parse_tree(&output)?
            .into_iter()
            .find(|entry| entry.path == path)
            .ok_or(BrowserError::NotFound)?;
        if entry.kind != "blob" {
            return Err(BrowserError::InvalidArgument);
        }
        if entry.size.is_none_or(|size| size > maximum as u64) {
            return Err(BrowserError::ResourceExhausted);
        }
        let contents = git(
            &repository,
            &["cat-file", "blob", &entry.object_id],
            maximum,
        )
        .await?;
        Ok((selected, entry, contents))
    }

    pub async fn commit_detail(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
        request: CommitDetailRequest<'_>,
    ) -> Result<(Branch, CommitDetail, bool), BrowserError> {
        let repository = self.repository(identity, id).await?;
        let selected = resolve_branch(&repository, request.branch).await?;
        validate_object_id(request.commit)?;
        if !is_ancestor(&repository, request.commit, &selected.commit).await? {
            return Err(BrowserError::NotFound);
        }
        let metadata = parse_commit_detail(
            &git_owned(
                &repository,
                &[
                    String::from("show"),
                    String::from("-s"),
                    String::from(
                        "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%s%x00%b",
                    ),
                    request.commit.to_owned(),
                ],
                65_536,
            )
            .await?,
        )?;
        let selected_parent = select_parent(&metadata.commit.parents, request.parent)?;
        let (mut files, truncated) =
            changed_files(&repository, request.commit, &selected_parent).await?;
        let has_more = files.len() > request.skip.saturating_add(request.limit);
        files = files
            .into_iter()
            .skip(request.skip)
            .take(request.limit)
            .collect();
        for file in &mut files {
            populate_hunks(&repository, request.commit, &selected_parent, file).await?;
        }
        let truncated = truncated || files.iter().any(|file| file.truncated);
        Ok((
            selected,
            CommitDetail {
                commit: metadata.commit,
                committer_name: metadata.committer_name,
                committer_email: metadata.committer_email,
                committed_at: metadata.committed_at,
                body: metadata.body,
                selected_parent,
                files,
                truncated,
            },
            has_more,
        ))
    }

    async fn repository(
        &self,
        identity: &AuthenticatedIdentity,
        id: Uuid,
    ) -> Result<std::path::PathBuf, BrowserError> {
        let mut tx = self.pool.begin().await.map_err(BrowserError::Persistence)?;
        sqlx::query("SELECT set_config('hephaestus.actor_id', $1, true), set_config('hephaestus.subject_type', 'user', true), set_config('hephaestus.request_id', $2, true), set_config('hephaestus.occurrence_id', $3, true)")
            .bind(identity.user_id.to_string()).bind(identity.request_id.to_string()).bind(identity.idempotency_id.to_string()).execute(&mut *tx).await.map_err(BrowserError::Persistence)?;
        let allowed: bool = sqlx::query_scalar("SELECT check_permission('user', hephaestus_actor_id(), 'can_read', 'repository', $1::text) = 1")
            .bind(id).fetch_one(&mut *tx).await.map_err(BrowserError::Persistence)?;
        tx.commit().await.map_err(BrowserError::Persistence)?;
        if !allowed {
            return Err(BrowserError::PermissionDenied);
        }
        let repository_id: RepositoryId = id
            .to_string()
            .parse()
            .map_err(|_| BrowserError::InvalidArgument)?;
        self.storage
            .validate_existing(repository_id)
            .await
            .map_err(BrowserError::Storage)
    }
}
