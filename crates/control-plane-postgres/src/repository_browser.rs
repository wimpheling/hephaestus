//! Authorized, bounded inspection of canonical bare Git repositories.

use forge_domain::RepositoryId;
use forge_service::GitStorage;
use identity_domain::AuthenticatedIdentity;
use sqlx::PgPool;
use std::{path::Path, process::Stdio, sync::Arc, time::Duration};
use tokio::{io::AsyncReadExt, process::Command};
use uuid::Uuid;

const GIT_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, thiserror::Error)]
pub enum BrowserError {
    #[error("repository access is denied")]
    PermissionDenied,
    #[error("repository object was not found")]
    NotFound,
    #[error("repository browser input is invalid")]
    InvalidArgument,
    #[error("repository browser limit exceeded")]
    ResourceExhausted,
    #[error("repository query failed")]
    Persistence(#[source] sqlx::Error),
    #[error("repository storage failed")]
    Storage(#[source] forge_service::GitStorageError),
    #[error("Git inspection failed")]
    Git,
}

#[derive(Clone)]
pub struct Branch {
    pub name: String,
    pub git_ref: String,
    pub commit: String,
    pub committed_at: i64,
    pub subject: String,
}

pub struct Commit {
    pub id: String,
    pub parents: Vec<String>,
    pub author_name: String,
    pub author_email: String,
    pub authored_at: i64,
    pub subject: String,
}

#[derive(Clone)]
pub struct TreeEntry {
    pub mode: String,
    pub kind: String,
    pub object_id: String,
    pub size: Option<u64>,
    pub path: String,
}

pub struct BrowserApplication {
    pool: PgPool,
    storage: Arc<GitStorage>,
}

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

async fn resolve_branch(repository: &Path, requested: &str) -> Result<Branch, BrowserError> {
    if requested.is_empty() || requested.len() > 255 || !requested.is_ascii() {
        return Err(BrowserError::InvalidArgument);
    }
    let output = git(
        repository,
        &[
            "for-each-ref",
            "--sort=refname",
            "--format=%(refname)%00%(objectname)%00%(committerdate:unix)%00%(subject)",
            "refs/heads/",
        ],
        1_048_576,
    )
    .await?;
    parse_branches(&output)?
        .into_iter()
        .find(|branch| branch.name == requested)
        .ok_or(BrowserError::NotFound)
}

async fn git(repository: &Path, args: &[&str], maximum: usize) -> Result<Vec<u8>, BrowserError> {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C.UTF-8");
    let mut child = command.spawn().map_err(|_| BrowserError::Git)?;
    let mut stdout = child
        .stdout
        .take()
        .ok_or(BrowserError::Git)?
        .take(u64::try_from(maximum).map_err(|_| BrowserError::ResourceExhausted)? + 1);
    let mut output = Vec::new();
    tokio::time::timeout(GIT_TIMEOUT, stdout.read_to_end(&mut output))
        .await
        .map_err(|_| BrowserError::Git)?
        .map_err(|_| BrowserError::Git)?;
    if output.len() > maximum {
        let _ = child.kill().await;
        return Err(BrowserError::ResourceExhausted);
    }
    let status = tokio::time::timeout(GIT_TIMEOUT, child.wait())
        .await
        .map_err(|_| BrowserError::Git)?
        .map_err(|_| BrowserError::Git)?;
    if status.success() {
        Ok(output)
    } else {
        Err(BrowserError::Git)
    }
}

fn parse_branches(output: &[u8]) -> Result<Vec<Branch>, BrowserError> {
    let text = std::str::from_utf8(output).map_err(|_| BrowserError::Git)?;
    text.lines()
        .map(|line| {
            let mut fields = line.splitn(4, '\0');
            let git_ref = fields.next().ok_or(BrowserError::Git)?.to_owned();
            let name = git_ref
                .strip_prefix("refs/heads/")
                .filter(|name| !name.is_empty())
                .ok_or(BrowserError::Git)?
                .to_owned();
            let commit = fields.next().ok_or(BrowserError::Git)?.to_owned();
            let committed_at = fields
                .next()
                .ok_or(BrowserError::Git)?
                .parse()
                .map_err(|_| BrowserError::Git)?;
            let subject = fields.next().ok_or(BrowserError::Git)?.to_owned();
            Ok(Branch {
                name,
                git_ref,
                commit,
                committed_at,
                subject,
            })
        })
        .collect()
}

fn parse_commits(output: &[u8]) -> Result<Vec<Commit>, BrowserError> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() % 6 != 0 {
        return Err(BrowserError::Git);
    }
    fields
        .chunks_exact(6)
        .map(|fields| {
            Ok(Commit {
                id: text(fields[0])?,
                parents: text(fields[1])?
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                author_name: text(fields[2])?,
                author_email: text(fields[3])?,
                authored_at: text(fields[4])?.parse().map_err(|_| BrowserError::Git)?,
                subject: text(fields[5])?,
            })
        })
        .collect()
}

fn parse_tree(output: &[u8]) -> Result<Vec<TreeEntry>, BrowserError> {
    output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .map(|record| {
            let separator = record
                .iter()
                .position(|byte| *byte == b'\t')
                .ok_or(BrowserError::Git)?;
            let (metadata, path_with_separator) = record.split_at(separator);
            let path = path_with_separator.get(1..).ok_or(BrowserError::Git)?;
            let metadata = text(metadata)?;
            let mut fields = metadata.split_whitespace();
            let mode = fields.next().ok_or(BrowserError::Git)?.to_owned();
            let kind = fields.next().ok_or(BrowserError::Git)?.to_owned();
            let object_id = fields.next().ok_or(BrowserError::Git)?.to_owned();
            let size = match fields.next().ok_or(BrowserError::Git)? {
                "-" => None,
                value => Some(value.parse().map_err(|_| BrowserError::Git)?),
            };
            Ok(TreeEntry {
                mode,
                kind,
                object_id,
                size,
                path: text(path)?,
            })
        })
        .collect()
}

fn text(value: &[u8]) -> Result<String, BrowserError> {
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| BrowserError::Git)
}

fn validate_path(path: &str) -> Result<(), BrowserError> {
    if path.is_empty()
        || path.len() > 4_096
        || path.contains('\0')
        || path.contains('\\')
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        Err(BrowserError::InvalidArgument)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_commits, parse_tree, validate_path};

    #[test]
    fn rejects_traversal_and_malformed_git_records() {
        assert!(validate_path("src/lib.rs").is_ok());
        assert!(validate_path("../secret").is_err());
        assert!(parse_commits(b"incomplete\0").is_err());
        assert!(parse_tree(b"100644 blob abc 1 missing-tab\0").is_err());
    }

    #[test]
    fn parses_root_commit_with_empty_parent_field() {
        let commits =
            parse_commits(b"abc\0\0Root Author\0root@example.test\x001700000000\0initial\0")
                .expect("root commit should parse");
        assert_eq!(commits.len(), 1);
        assert!(commits[0].parents.is_empty());
        assert_eq!(commits[0].subject, "initial");
    }
}
