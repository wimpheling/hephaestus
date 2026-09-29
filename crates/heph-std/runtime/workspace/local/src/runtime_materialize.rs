use crate::common::{
    LocalWorkspaceConfig, LocalWorkspaceError, OsStr, Path, RUNTIME_GIT_LOOPBACK_PORT,
    RuntimeGitWorkspaceRequest, ensure_workspace_path, fs, git_worktree_optional,
    git_worktree_output, git_worktree_text, io_error, sha256, utf8_path, validate_repository,
    write_owner_marker,
};

pub fn materialize_runtime_git(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    request: &RuntimeGitWorkspaceRequest,
    temporary: &Path,
    active: &Path,
) -> Result<(String, String), LocalWorkspaceError> {
    validate_repository(config, repository)?;
    ensure_workspace_path(config, temporary, "active")?;
    ensure_workspace_path(config, active, "active")?;
    if temporary.exists() || active.exists() {
        return Err(LocalWorkspaceError::State(String::from(
            "runtime Git workspace path already exists",
        )));
    }
    fs::create_dir(temporary).map_err(io_error)?;
    let owner = active
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("runtime owner is invalid")))?;
    write_owner_marker(temporary, owner)?;
    let repository_text = utf8_path(repository)?;
    git_worktree_output(
        config,
        temporary,
        &["init", "--initial-branch=runtime-target"],
    )?;
    git_worktree_output(
        config,
        temporary,
        &[
            "fetch",
            "--no-tags",
            "--no-write-fetch-head",
            &repository_text,
            &request.target_commit,
        ],
    )?;
    let commit_object = [request.target_commit.as_str(), "^{commit}"].concat();
    let fetched = git_worktree_text(
        config,
        temporary,
        &["rev-parse", "--verify", &commit_object],
    )?;
    if fetched != request.target_commit {
        return Err(LocalWorkspaceError::Integrity(String::from(
            "runtime Git fetch did not resolve the immutable target commit",
        )));
    }
    git_worktree_output(
        config,
        temporary,
        &["update-ref", &request.target_ref, &request.target_commit],
    )?;
    git_worktree_output(
        config,
        temporary,
        &["symbolic-ref", "HEAD", &request.target_ref],
    )?;
    git_worktree_output(
        config,
        temporary,
        &["read-tree", "--reset", "-u", &request.target_ref],
    )?;
    let remote_url =
        vm_trait::RuntimeGitBridge::new(request.repository_id, RUNTIME_GIT_LOOPBACK_PORT)
            .remote_url();
    git_worktree_output(config, temporary, &["remote", "add", "origin", &remote_url])?;
    let head = git_worktree_text(config, temporary, &["rev-parse", "HEAD"])?;
    if head != request.target_commit {
        return Err(LocalWorkspaceError::Integrity(String::from(
            "runtime Git checkout changed the immutable target commit",
        )));
    }
    let symbolic_head = git_worktree_text(config, temporary, &["symbolic-ref", "HEAD"])?;
    if symbolic_head != request.target_ref {
        return Err(LocalWorkspaceError::Integrity(String::from(
            "runtime Git worktree HEAD is not the authorized branch",
        )));
    }
    let configured_origin = git_worktree_text(config, temporary, &["remote", "get-url", "origin"])?;
    if configured_origin != remote_url
        || git_worktree_optional(
            config,
            temporary,
            &["config", "--local", "--get-regexp", "^remote\\."],
        )?
        .is_none()
    {
        return Err(LocalWorkspaceError::Integrity(String::from(
            "runtime Git origin is not the token-free bridge URL",
        )));
    }
    let alternates = temporary.join(".git/objects/info/alternates");
    if alternates.exists() {
        return Err(LocalWorkspaceError::Integrity(String::from(
            "runtime Git worktree contains an object alternates file",
        )));
    }
    let tree_object = [request.target_commit.as_str(), "^{", "tree}"].concat();
    let tree = git_worktree_text(config, temporary, &["rev-parse", &tree_object])?;
    let manifest_hash =
        sha256(format!("{}\n{}\n", request.target_ref, request.target_commit).as_bytes());
    fs::rename(temporary, active).map_err(io_error)?;
    Ok((tree, manifest_hash))
}
