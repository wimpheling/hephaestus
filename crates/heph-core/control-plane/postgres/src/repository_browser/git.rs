use std::{path::Path, process::Stdio, time::Duration};

use tokio::{io::AsyncReadExt, process::Command};

use super::{Branch, BrowserError, Commit, TreeEntry};

pub(super) const GIT_TIMEOUT: Duration = Duration::from_secs(5);

pub(super) async fn resolve_branch(
    repository: &Path,
    requested: &str,
) -> Result<Branch, BrowserError> {
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

pub(super) async fn git(
    repository: &Path,
    args: &[&str],
    maximum: usize,
) -> Result<Vec<u8>, BrowserError> {
    let mut command = git_command(repository);
    command.args(args);
    read_git_output(command, maximum).await
}

pub(super) async fn git_owned(
    repository: &Path,
    args: &[String],
    maximum: usize,
) -> Result<Vec<u8>, BrowserError> {
    let mut command = git_command(repository);
    command.args(args);
    read_git_output(command, maximum).await
}

pub(super) fn git_command(repository: &Path) -> Command {
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

pub(super) fn parse_branches(output: &[u8]) -> Result<Vec<Branch>, BrowserError> {
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

pub(super) fn parse_commits(output: &[u8]) -> Result<Vec<Commit>, BrowserError> {
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

pub(super) fn parse_tree(output: &[u8]) -> Result<Vec<TreeEntry>, BrowserError> {
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

pub(super) fn text(value: &[u8]) -> Result<String, BrowserError> {
    std::str::from_utf8(value)
        .map(str::to_owned)
        .map_err(|_| BrowserError::Git)
}

pub(super) fn validate_path(path: &str) -> Result<(), BrowserError> {
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
