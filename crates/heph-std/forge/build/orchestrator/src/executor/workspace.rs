use super::{
    BuildExecutionError, BuildExecutorConfig, BuildInput, Component, MAX_SOURCE_BYTES,
    MAX_SOURCE_ENTRIES, OsStr, Path, PathBuf, Uuid, filesystem,
};
use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    process::Command,
};

pub(super) struct PreparedBuildWorkspace {
    pub(super) root: PathBuf,
    pub(super) source: PathBuf,
    pub(super) output: PathBuf,
}

pub(super) fn prepare_workspace(
    config: &BuildExecutorConfig,
    input: &BuildInput,
    active: &Path,
) -> Result<PreparedBuildWorkspace, BuildExecutionError> {
    if active.exists() {
        return Err(BuildExecutionError::UnsafeWorkspace);
    }
    let repository = config
        .repository_root
        .join(format!("{}.git", input.repository_id));
    let repository = fs::canonicalize(repository).map_err(filesystem)?;
    if repository.parent() != Some(config.repository_root.as_path()) || !repository.is_dir() {
        return Err(BuildExecutionError::UnsafeRepository);
    }
    let staging = config
        .workspace_root
        .join(format!(".prepare-{}", Uuid::new_v4()));
    fs::create_dir(&staging).map_err(filesystem)?;
    fs::set_permissions(&staging, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    let source = staging.join("source");
    let output = staging.join("output");
    fs::create_dir(&source).map_err(filesystem)?;
    fs::create_dir(&output).map_err(filesystem)?;
    if let Err(error) = materialize_source(config, &repository, &input.source_commit, &source) {
        drop(fs::remove_dir_all(&staging));
        return Err(error);
    }
    fs::set_permissions(&output, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    fs::rename(&staging, active).map_err(filesystem)?;
    Ok(PreparedBuildWorkspace {
        root: active.to_path_buf(),
        source: active.join("source"),
        output: active.join("output"),
    })
}

fn materialize_source(
    config: &BuildExecutorConfig,
    repository: &Path,
    commit: &str,
    source: &Path,
) -> Result<(), BuildExecutionError> {
    let tree = git_output(
        config,
        repository,
        &["ls-tree", "-rz", "-r", "--full-tree", commit],
    )?;
    let records = tree
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
        .collect::<Vec<_>>();
    if records.len() > MAX_SOURCE_ENTRIES {
        return Err(BuildExecutionError::SourceQuota);
    }
    let mut total = 0_u64;
    for record in records {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or(BuildExecutionError::InvalidGitTree)?;
        let metadata =
            std::str::from_utf8(&record[..tab]).map_err(|_| BuildExecutionError::InvalidGitTree)?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| BuildExecutionError::InvalidGitTree)?;
        validate_relative_path(path)?;
        let mut fields = metadata.split_ascii_whitespace();
        let mode =
            u32::from_str_radix(fields.next().ok_or(BuildExecutionError::InvalidGitTree)?, 8)
                .map_err(|_| BuildExecutionError::InvalidGitTree)?;
        if fields.next() != Some("blob") || !matches!(mode, 0o100_644 | 0o100_755) {
            return Err(BuildExecutionError::UnsupportedSourceObject);
        }
        let object = fields.next().ok_or(BuildExecutionError::InvalidGitTree)?;
        let bytes = git_output(config, repository, &["cat-file", "blob", object])?;
        total = total
            .checked_add(u64::try_from(bytes.len()).map_err(|_| BuildExecutionError::SourceQuota)?)
            .ok_or(BuildExecutionError::SourceQuota)?;
        if total > MAX_SOURCE_BYTES {
            return Err(BuildExecutionError::SourceQuota);
        }
        let destination = source.join(path);
        fs::create_dir_all(
            destination
                .parent()
                .ok_or(BuildExecutionError::InvalidGitTree)?,
        )
        .map_err(filesystem)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if mode == 0o100_755 { 0o500 } else { 0o400 })
            .open(destination)
            .map_err(filesystem)?;
        file.write_all(&bytes).map_err(filesystem)?;
        file.flush().map_err(filesystem)?;
    }
    seal_source_directories(source)
}

fn seal_source_directories(path: &Path) -> Result<(), BuildExecutionError> {
    for entry in fs::read_dir(path).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            seal_source_directories(&entry.path())?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o500)).map_err(filesystem)
}

fn git_output(
    config: &BuildExecutorConfig,
    repository: &Path,
    arguments: &[&str],
) -> Result<Vec<u8>, BuildExecutionError> {
    let output = Command::new(&config.git_binary)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .arg("--git-dir")
        .arg(repository)
        .args(arguments)
        .output()
        .map_err(filesystem)?;
    if !output.status.success() {
        return Err(BuildExecutionError::Git);
    }
    Ok(output.stdout)
}

fn validate_relative_path(value: &str) -> Result<(), BuildExecutionError> {
    if value.is_empty() || value.len() > 1_024 || value.contains('\\') {
        return Err(BuildExecutionError::InvalidGitTree);
    }
    let path = Path::new(value);
    if path.is_absolute()
        || path.components().any(|component| {
            !matches!(component, Component::Normal(_))
                || matches!(component, Component::Normal(name) if name == OsStr::new(".git"))
        })
    {
        return Err(BuildExecutionError::InvalidGitTree);
    }
    Ok(())
}
