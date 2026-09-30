use std::path::Path;

use super::{
    BrowserError, Commit,
    diff_parse::{truncate_text, validate_object_id},
    git::{GIT_TIMEOUT, git_command, text},
};

pub(super) struct ParsedCommitDetail {
    pub(super) commit: Commit,
    pub(super) committer_name: String,
    pub(super) committer_email: String,
    pub(super) committed_at: i64,
    pub(super) body: String,
}

pub(super) async fn is_ancestor(
    repository: &Path,
    commit: &str,
    branch: &str,
) -> Result<bool, BrowserError> {
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
pub(super) fn parse_commit_detail(output: &[u8]) -> Result<ParsedCommitDetail, BrowserError> {
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

pub(super) fn select_parent(parents: &[String], requested: &str) -> Result<String, BrowserError> {
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
