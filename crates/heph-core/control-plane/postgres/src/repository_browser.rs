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

pub struct CommitDetail {
    pub commit: Commit,
    pub committer_name: String,
    pub committer_email: String,
    pub committed_at: i64,
    pub body: String,
    pub selected_parent: String,
    pub files: Vec<DiffFile>,
    pub truncated: bool,
}

pub struct CommitDetailRequest<'a> {
    pub branch: &'a str,
    pub commit: &'a str,
    pub parent: &'a str,
    pub skip: usize,
    pub limit: usize,
}

pub struct DiffFile {
    pub path: String,
    pub previous_path: String,
    pub state: DiffFileState,
    pub additions: u64,
    pub deletions: u64,
    pub hunks: Vec<DiffHunk>,
    pub truncated: bool,
}

#[derive(Clone, Copy)]
pub enum DiffFileState {
    Added,
    Deleted,
    Modified,
    Renamed,
    Binary,
    Truncated,
    Unavailable,
}

pub struct DiffHunk {
    pub old_start: u32,
    pub old_lines: u32,
    pub new_start: u32,
    pub new_lines: u32,
    pub lines: Vec<DiffLine>,
    pub truncated: bool,
}

pub struct DiffLine {
    pub kind: DiffLineKind,
    pub old_line: Option<u32>,
    pub new_line: Option<u32>,
    pub text: String,
}

#[derive(Clone, Copy)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
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

const MAX_CHANGED_FILES: usize = 100;
const MAX_HUNKS_PER_FILE: usize = 50;
const MAX_LINES_PER_HUNK: usize = 500;
const MAX_DIFF_BYTES_PER_FILE: usize = 131_072;

struct ParsedCommitDetail {
    commit: Commit,
    committer_name: String,
    committer_email: String,
    committed_at: i64,
    body: String,
}

async fn is_ancestor(repository: &Path, commit: &str, branch: &str) -> Result<bool, BrowserError> {
    let mut command = git_command(repository);
    let status = tokio::time::timeout(
        GIT_TIMEOUT,
        command
            .args(["merge-base", "--is-ancestor", commit, branch])
            .status(),
    )
    .await
    .map_err(|_| BrowserError::Git)?
    .map_err(|_| BrowserError::Git)?;
    Ok(status.success())
}

async fn changed_files(
    repository: &Path,
    commit: &str,
    parent: &str,
) -> Result<(Vec<DiffFile>, bool), BrowserError> {
    let args = if parent.is_empty() {
        vec![
            String::from("diff-tree"),
            String::from("--root"),
            String::from("--no-commit-id"),
            String::from("--find-renames"),
            String::from("-r"),
            String::from("--name-status"),
            String::from("-z"),
            commit.to_owned(),
        ]
    } else {
        vec![
            String::from("diff"),
            String::from("--find-renames"),
            String::from("--name-status"),
            String::from("-z"),
            parent.to_owned(),
            commit.to_owned(),
        ]
    };
    let output = git_owned(repository, &args, 1_048_576).await?;
    let mut values = Vec::new();
    let mut fields = output.split(|byte| *byte == 0);
    loop {
        let Some(status) = fields.next() else {
            break;
        };
        if status.is_empty() {
            continue;
        }
        let status = text(status)?;
        let first_path = text(fields.next().ok_or(BrowserError::Git)?)?;
        let (path, previous_path, state) = match status.as_bytes().first() {
            Some(b'A') => (first_path, String::new(), DiffFileState::Added),
            Some(b'D') => (first_path, String::new(), DiffFileState::Deleted),
            Some(b'M' | b'T' | b'C') => (first_path, String::new(), DiffFileState::Modified),
            Some(b'R') => (
                text(fields.next().ok_or(BrowserError::Git)?)?,
                first_path,
                DiffFileState::Renamed,
            ),
            _ => return Err(BrowserError::Git),
        };
        if values.len() == MAX_CHANGED_FILES {
            return Ok((values, true));
        }
        values.push(DiffFile {
            path,
            previous_path,
            state,
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
            truncated: false,
        });
    }
    Ok((values, false))
}

async fn populate_hunks(
    repository: &Path,
    commit: &str,
    parent: &str,
    file: &mut DiffFile,
) -> Result<(), BrowserError> {
    let args = if parent.is_empty() {
        vec![
            String::from("diff-tree"),
            String::from("--root"),
            String::from("--no-commit-id"),
            String::from("--no-ext-diff"),
            String::from("--no-color"),
            String::from("--unified=3"),
            commit.to_owned(),
            String::from("--"),
            file.path.clone(),
        ]
    } else {
        vec![
            String::from("diff"),
            String::from("--no-ext-diff"),
            String::from("--no-color"),
            String::from("--unified=3"),
            parent.to_owned(),
            commit.to_owned(),
            String::from("--"),
            file.path.clone(),
        ]
    };
    let output = match git_owned(repository, &args, MAX_DIFF_BYTES_PER_FILE).await {
        Ok(output) => output,
        Err(BrowserError::ResourceExhausted) => {
            file.state = DiffFileState::Truncated;
            file.truncated = true;
            return Ok(());
        }
        // One malformed or temporarily unavailable object must not turn an
        // otherwise authorized, bounded commit inspection into an error page.
        Err(BrowserError::Git | BrowserError::Storage(_)) => {
            file.state = DiffFileState::Unavailable;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let Ok(source) = String::from_utf8(output) else {
        file.state = DiffFileState::Binary;
        return Ok(());
    };
    if source.contains("Binary files ") || source.contains("GIT binary patch") {
        file.state = DiffFileState::Binary;
        return Ok(());
    }
    let (hunks, additions, deletions, truncated) = match parse_hunks(&source) {
        Ok(parsed) => parsed,
        Err(BrowserError::Git) => {
            file.state = DiffFileState::Unavailable;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    file.hunks = hunks;
    file.additions = additions;
    file.deletions = deletions;
    file.truncated = truncated;
    if truncated {
        file.state = DiffFileState::Truncated;
    }
    Ok(())
}

fn parse_commit_detail(output: &[u8]) -> Result<ParsedCommitDetail, BrowserError> {
    let mut fields = output.split(|byte| *byte == 0).collect::<Vec<_>>();
    if fields.last().is_some_and(|field| field.is_empty()) {
        fields.pop();
    }
    if fields.len() != 10 {
        return Err(BrowserError::Git);
    }
    let parents = text(fields[1])?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    Ok(ParsedCommitDetail {
        commit: Commit {
            id: text(fields[0])?,
            parents,
            author_name: text(fields[2])?,
            author_email: text(fields[3])?,
            authored_at: text(fields[4])?.parse().map_err(|_| BrowserError::Git)?,
            subject: text(fields[8])?,
        },
        committer_name: text(fields[5])?,
        committer_email: text(fields[6])?,
        committed_at: text(fields[7])?.parse().map_err(|_| BrowserError::Git)?,
        body: truncate_text(text(fields[9])?, 16_384),
    })
}

fn select_parent(parents: &[String], requested: &str) -> Result<String, BrowserError> {
    if requested.is_empty() {
        return Ok(parents.first().cloned().unwrap_or_default());
    }
    validate_object_id(requested)?;
    parents
        .iter()
        .find(|parent| parent.as_str() == requested)
        .cloned()
        .ok_or(BrowserError::NotFound)
}

fn parse_hunks(source: &str) -> Result<(Vec<DiffHunk>, u64, u64, bool), BrowserError> {
    let mut hunks = Vec::new();
    let mut active = None;
    let mut additions = 0;
    let mut deletions = 0;
    let mut truncated = false;
    for line in source.lines() {
        if let Some(header) = line.strip_prefix("@@ ") {
            if let Some(hunk) = active.take() {
                hunks.push(hunk);
            }
            if hunks.len() == MAX_HUNKS_PER_FILE {
                truncated = true;
                break;
            }
            active = Some(parse_hunk_header(header)?);
            continue;
        }
        let Some(hunk) = active.as_mut() else {
            continue;
        };
        if hunk.lines.len() == MAX_LINES_PER_HUNK {
            hunk.truncated = true;
            truncated = true;
            continue;
        }
        let (kind, text) = match line.as_bytes().first() {
            Some(b' ') => (DiffLineKind::Context, &line[1..]),
            Some(b'+') if !line.starts_with("+++") => {
                additions += 1;
                (DiffLineKind::Added, &line[1..])
            }
            Some(b'-') if !line.starts_with("---") => {
                deletions += 1;
                (DiffLineKind::Removed, &line[1..])
            }
            _ => continue,
        };
        let old_line = match kind {
            DiffLineKind::Added => None,
            DiffLineKind::Context | DiffLineKind::Removed => {
                Some(hunk.old_start + hunk.old_lines_seen)
            }
        };
        let new_line = match kind {
            DiffLineKind::Removed => None,
            DiffLineKind::Context | DiffLineKind::Added => {
                Some(hunk.new_start + hunk.new_lines_seen)
            }
        };
        if !matches!(kind, DiffLineKind::Added) {
            hunk.old_lines_seen += 1;
        }
        if !matches!(kind, DiffLineKind::Removed) {
            hunk.new_lines_seen += 1;
        }
        hunk.lines.push(DiffLine {
            kind,
            old_line,
            new_line,
            text: text.to_owned(),
        });
    }
    if let Some(hunk) = active {
        hunks.push(hunk);
    }
    Ok((
        hunks
            .into_iter()
            .map(|hunk| DiffHunk {
                old_start: hunk.old_start,
                old_lines: hunk.old_lines,
                new_start: hunk.new_start,
                new_lines: hunk.new_lines,
                lines: hunk.lines,
                truncated: hunk.truncated,
            })
            .collect(),
        additions,
        deletions,
        truncated,
    ))
}

struct ActiveHunk {
    old_start: u32,
    old_lines: u32,
    new_start: u32,
    new_lines: u32,
    old_lines_seen: u32,
    new_lines_seen: u32,
    lines: Vec<DiffLine>,
    truncated: bool,
}

fn parse_hunk_header(value: &str) -> Result<ActiveHunk, BrowserError> {
    let (range, _context) = value.split_once(" @@").ok_or(BrowserError::Git)?;
    let mut ranges = range.split_whitespace();
    let old = parse_hunk_range(ranges.next().ok_or(BrowserError::Git)?, '-')?;
    let new = parse_hunk_range(ranges.next().ok_or(BrowserError::Git)?, '+')?;
    Ok(ActiveHunk {
        old_start: old.0,
        old_lines: old.1,
        new_start: new.0,
        new_lines: new.1,
        old_lines_seen: 0,
        new_lines_seen: 0,
        lines: Vec::new(),
        truncated: false,
    })
}

fn parse_hunk_range(value: &str, prefix: char) -> Result<(u32, u32), BrowserError> {
    let value = value.strip_prefix(prefix).ok_or(BrowserError::Git)?;
    let (start, count) = value.split_once(',').unwrap_or((value, "1"));
    Ok((
        start.parse().map_err(|_| BrowserError::Git)?,
        count.parse().map_err(|_| BrowserError::Git)?,
    ))
}

fn truncate_text(mut value: String, maximum: usize) -> String {
    if value.len() > maximum {
        value.truncate(maximum);
    }
    value
}

fn validate_object_id(value: &str) -> Result<(), BrowserError> {
    if (value.len() == 40 || value.len() == 64)
        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(BrowserError::InvalidArgument)
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
    let mut command = git_command(repository);
    command.args(args);
    read_git_output(command, maximum).await
}

async fn git_owned(
    repository: &Path,
    args: &[String],
    maximum: usize,
) -> Result<Vec<u8>, BrowserError> {
    let mut command = git_command(repository);
    command.args(args);
    read_git_output(command, maximum).await
}

fn git_command(repository: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repository)
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
    command
}

async fn read_git_output(mut command: Command, maximum: usize) -> Result<Vec<u8>, BrowserError> {
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
    use super::{
        DiffFileState, DiffLineKind, MAX_LINES_PER_HUNK, changed_files, parse_commits, parse_hunks,
        parse_tree, populate_hunks, select_parent, validate_path,
    };
    use std::{fs, path::Path, process::Command};
    use tempfile::TempDir;

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

    #[test]
    fn parses_numbered_unified_hunks_without_interpreting_source_as_markup() {
        let (hunks, additions, deletions, truncated) = parse_hunks(
            "@@ -2,2 +2,3 @@ fn sample() {\n unchanged\n-old <script>\n+new <script>\n+extra\n",
        )
        .expect("valid unified diff");
        assert!(!truncated);
        assert_eq!((additions, deletions), (2, 1));
        assert!(matches!(hunks[0].lines[1].kind, DiffLineKind::Removed));
        assert_eq!(hunks[0].lines[2].text, "new <script>");
        assert_eq!(hunks[0].lines[2].new_line, Some(3));
    }

    #[test]
    fn bounds_one_large_hunk_without_losing_its_safe_prefix() {
        let source = format!(
            "@@ -1,0 +1,{} @@\n{}",
            MAX_LINES_PER_HUNK + 1,
            "+safe\n".repeat(MAX_LINES_PER_HUNK + 1)
        );
        let (hunks, additions, _deletions, truncated) =
            parse_hunks(&source).expect("valid bounded unified diff");
        assert!(truncated);
        assert!(hunks[0].truncated);
        assert_eq!(hunks[0].lines.len(), MAX_LINES_PER_HUNK);
        assert_eq!(
            additions,
            u64::try_from(MAX_LINES_PER_HUNK).expect("fits u64")
        );
    }

    #[test]
    fn root_and_merge_parent_selection_are_deterministic() {
        assert_eq!(select_parent(&[], "").expect("root parent"), "");
        assert_eq!(
            select_parent(&["a".repeat(40), "b".repeat(40)], "").expect("first parent"),
            "a".repeat(40)
        );
        assert!(select_parent(&["a".repeat(40)], &"b".repeat(40)).is_err());
    }

    #[tokio::test]
    async fn real_git_diff_marks_rename_and_binary_without_exposing_binary_bytes() {
        let fixture = repository_fixture();
        let repository = fixture.path().join(".git");
        let root = git_at(fixture.path(), ["rev-parse", "HEAD"]);

        fs::rename(
            fixture.path().join("recipe.txt"),
            fixture.path().join("renamed-recipe.txt"),
        )
        .expect("rename fixture file");
        fs::write(fixture.path().join("renamed-recipe.txt"), "soup\nstew\n")
            .expect("modify renamed fixture");
        fs::write(fixture.path().join("image.bin"), [0, 159, 146, 150]).expect("binary fixture");
        git_at(fixture.path(), ["add", "-A"]);
        git_at(
            fixture.path(),
            ["commit", "--quiet", "-m", "rename and binary"],
        );
        let commit = git_at(fixture.path(), ["rev-parse", "HEAD"]);

        let (mut files, truncated) = changed_files(&repository, &commit, &root)
            .await
            .expect("bounded file list");
        assert!(!truncated);
        for file in &mut files {
            populate_hunks(&repository, &commit, &root, file)
                .await
                .expect("safe per-file inspection");
        }

        let renamed = files
            .iter()
            .find(|file| file.path == "renamed-recipe.txt")
            .expect("renamed file summary");
        assert!(matches!(renamed.state, DiffFileState::Renamed));
        assert_eq!(renamed.previous_path, "recipe.txt");
        let binary = files
            .iter()
            .find(|file| file.path == "image.bin")
            .expect("binary file summary");
        assert!(matches!(binary.state, DiffFileState::Binary));
        assert!(binary.hunks.is_empty());

        fs::remove_file(fixture.path().join("renamed-recipe.txt")).expect("delete fixture");
        git_at(fixture.path(), ["add", "-A"]);
        git_at(fixture.path(), ["commit", "--quiet", "-m", "delete recipe"]);
        let deletion = git_at(fixture.path(), ["rev-parse", "HEAD"]);
        let (files, truncated) = changed_files(&repository, &deletion, &commit)
            .await
            .expect("bounded deletion list");
        assert!(!truncated);
        assert!(files.iter().any(|file| {
            file.path == "renamed-recipe.txt" && matches!(file.state, DiffFileState::Deleted)
        }));
    }

    fn repository_fixture() -> TempDir {
        let fixture = tempfile::tempdir().expect("temporary Git repository");
        git_at(fixture.path(), ["init", "--quiet"]);
        git_at(fixture.path(), ["config", "user.name", "Hephaestus test"]);
        git_at(
            fixture.path(),
            ["config", "user.email", "test@hephaestus.invalid"],
        );
        fs::write(fixture.path().join("recipe.txt"), "soup\n").expect("root fixture");
        git_at(fixture.path(), ["add", "recipe.txt"]);
        git_at(fixture.path(), ["commit", "--quiet", "-m", "root recipe"]);
        fixture
    }

    fn git_at<const N: usize>(directory: &Path, arguments: [&str; N]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(arguments)
            .output()
            .expect("run Git fixture command");
        assert!(
            output.status.success(),
            "Git fixture command failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout)
            .expect("Git fixture output")
            .trim()
            .to_owned()
    }
}
