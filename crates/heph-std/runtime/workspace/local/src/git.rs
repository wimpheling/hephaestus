use crate::common::{
    Command, LocalWorkspaceConfig, LocalWorkspaceError, OsStr, Path, Stdio, Write, fs, io_error,
};

pub fn git_optional_text(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
) -> Result<Option<String>, LocalWorkspaceError> {
    let mut command = git_command(config, repository, arguments);
    let output = command.output().map_err(io_error)?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(Some(text.trim().to_owned()))
}

pub fn git_text(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Result<String, LocalWorkspaceError> {
    let bytes = git_output(config, repository, arguments, input, environment)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(text.trim().to_owned())
}

pub fn git_output(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
    input: Option<&[u8]>,
    environment: &[(&str, &str)],
) -> Result<Vec<u8>, LocalWorkspaceError> {
    let mut command = git_command(config, repository, arguments);
    for (key, value) in environment {
        command.env(key, value);
    }
    if input.is_some() {
        command.stdin(Stdio::piped());
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(io_error)?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or_else(|| LocalWorkspaceError::Git(String::from("Git stdin was not piped")))?
            .write_all(input)
            .map_err(io_error)?;
    }
    let output = child.wait_with_output().map_err(io_error)?;
    if !output.status.success() {
        return Err(LocalWorkspaceError::Git(format!(
            "Git {:?} failed: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub fn git_worktree_output(
    config: &LocalWorkspaceConfig,
    worktree: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, LocalWorkspaceError> {
    let mut command = git_worktree_command(config, worktree, arguments);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let output = command.output().map_err(io_error)?;
    if !output.status.success() {
        return Err(LocalWorkspaceError::Git(format!(
            "Git {:?} failed: {}",
            arguments,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub fn git_worktree_text(
    config: &LocalWorkspaceConfig,
    worktree: &Path,
    arguments: &[&str],
) -> Result<String, LocalWorkspaceError> {
    let bytes = git_worktree_output(config, worktree, arguments)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(text.trim().to_owned())
}

pub fn git_worktree_optional(
    config: &LocalWorkspaceConfig,
    worktree: &Path,
    arguments: &[&str],
) -> Result<Option<String>, LocalWorkspaceError> {
    let mut command = git_worktree_command(config, worktree, arguments);
    let output = command.output().map_err(io_error)?;
    if !output.status.success() {
        return Ok(None);
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| LocalWorkspaceError::Git(String::from("Git output is not UTF-8")))?;
    Ok(Some(text.trim().to_owned()))
}

pub fn git_worktree_command(
    config: &LocalWorkspaceConfig,
    worktree: &Path,
    arguments: &[&str],
) -> Command {
    let mut command = Command::new(&config.git_binary);
    command
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg("-C")
        .arg(worktree)
        .args(arguments);
    command
}

pub fn git_command(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    arguments: &[&str],
) -> Command {
    let mut command = Command::new(&config.git_binary);
    command
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments);
    command
}

pub fn validate_repository(
    config: &LocalWorkspaceConfig,
    repository: &Path,
) -> Result<(), LocalWorkspaceError> {
    let parent = repository.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("repository path has no parent"))
    })?;
    let metadata = fs::symlink_metadata(repository).map_err(io_error)?;
    if parent != config.repository_root
        || repository.extension() != Some(OsStr::new("git"))
        || !metadata.file_type().is_dir()
    {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "repository path is outside the canonical layout",
        )));
    }
    Ok(())
}
