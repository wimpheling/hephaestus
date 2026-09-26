use super::{
    ApprovalDisposition, ApprovalProposal, ControlServiceError, RepositoryLocator, ReviewGit,
};
use forge_domain::{CommitSha, GitRef};
use std::{path::Path, process::Stdio, sync::Arc};
use tokio::process::Command;

/// Trusted host implementation of review result publication.
#[derive(Clone)]
pub struct GitReviewPublisher {
    locator: Arc<dyn RepositoryLocator>,
}

impl GitReviewPublisher {
    /// Creates a publisher over canonical repository path resolution.
    #[must_use]
    pub fn new(locator: Arc<dyn RepositoryLocator>) -> Self {
        Self { locator }
    }
}

#[async_trait::async_trait]
impl ReviewGit for GitReviewPublisher {
    async fn publish(
        &self,
        proposal: &ApprovalProposal,
    ) -> Result<ApprovalDisposition, ControlServiceError> {
        let repository = self
            .locator
            .locate(proposal.repository_id)
            .await
            .map_err(ControlServiceError::Storage)?;
        let target_ref = GitRef::parse(proposal.target_ref.clone())?;
        let result_ref = GitRef::parse(proposal.result_ref.clone())?;
        let input = CommitSha::parse(proposal.input_commit.clone())?;
        let result = CommitSha::parse(proposal.result_commit.clone())?;
        validate_result_provenance(&repository, &result_ref, &result, &input).await?;
        let current = resolve_ref(&repository, &target_ref).await?;
        if current.as_ref() == Some(&result) {
            return Ok(ApprovalDisposition::Approved);
        }
        if current.as_ref() != Some(&input) {
            return Ok(ApprovalDisposition::Conflicted);
        }
        cas_update_ref(&repository, &target_ref, &result, &input).await?;
        Ok(ApprovalDisposition::Approved)
    }
}

async fn validate_result_provenance(
    repository: &Path,
    result_ref: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let published = resolve_ref(repository, result_ref).await?;
    if published.as_ref() != Some(result) {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "controlled result ref does not point at the recorded result",
        )));
    }
    let parent = git_text(repository, &["rev-parse", &format!("{}^", result.as_str())]).await?;
    if parent != input.as_str() {
        return Err(ControlServiceError::InvalidResultProvenance(String::from(
            "result commit parent is not the exact input commit",
        )));
    }
    Ok(())
}

async fn resolve_ref(
    repository: &Path,
    git_ref: &GitRef,
) -> Result<Option<CommitSha>, ControlServiceError> {
    let output = git_output(repository, &["rev-parse", "--verify", git_ref.as_str()]).await?;
    if output.status.success() {
        let value = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        return Ok(Some(CommitSha::parse(value)?));
    }
    Ok(None)
}

async fn cas_update_ref(
    repository: &Path,
    target: &GitRef,
    result: &CommitSha,
    input: &CommitSha,
) -> Result<(), ControlServiceError> {
    let output = git_output(
        repository,
        &[
            "update-ref",
            target.as_str(),
            result.as_str(),
            input.as_str(),
        ],
    )
    .await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(())
}

async fn git_text(repository: &Path, arguments: &[&str]) -> Result<String, ControlServiceError> {
    let output = git_output(repository, arguments).await?;
    if !output.status.success() {
        return Err(ControlServiceError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

async fn git_output(
    repository: &Path,
    arguments: &[&str],
) -> Result<std::process::Output, ControlServiceError> {
    Command::new("git")
        .arg("--git-dir")
        .arg(repository)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(ControlServiceError::Io)
}
