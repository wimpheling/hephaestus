use super::ReceiveHookError;
use std::{
    ffi::OsStr,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

const MAX_INSPECTION_OUTPUT_BYTES: u64 = 64 * 1_024 * 1_024;

pub(super) fn canonical_ref(
    repository: &Path,
    reference: &str,
) -> Result<Option<String>, ReceiveHookError> {
    let output = Command::new("git")
        .args(["--git-dir"])
        .arg(repository)
        .args(["show-ref", "--verify", "--hash", reference])
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .map_err(ReceiveHookError::Io)?;
    if output.status.success() {
        let value = std::str::from_utf8(&output.stdout)
            .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)?
            .trim();
        if !valid_object_name(value) || is_zero_object_name(value) {
            return Err(ReceiveHookError::InvalidRepositoryFacts);
        }
        return Ok(Some(value.to_owned()));
    }
    if output.status.code() == Some(1) {
        return Ok(None);
    }
    Err(ReceiveHookError::GitCommand)
}

pub(super) fn ensure_object_exists(
    repository: &Path,
    quarantine: &Path,
    object: &str,
) -> Result<(), ReceiveHookError> {
    let output = git_status(repository, quarantine, ["cat-file", "-e", object])?;
    if output.success() {
        Ok(())
    } else {
        Err(ReceiveHookError::InvalidRepositoryFacts)
    }
}

pub(super) fn peel_commit(
    repository: &Path,
    quarantine: &Path,
    object: &str,
) -> Result<String, ReceiveHookError> {
    let expression = format!("{object}^{{commit}}");
    let output = git_output(
        repository,
        quarantine,
        ["rev-parse", "--verify", &expression],
    )?;
    let value = std::str::from_utf8(&output)
        .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)?
        .trim();
    if !valid_object_name(value) || is_zero_object_name(value) {
        return Err(ReceiveHookError::InvalidRepositoryFacts);
    }
    Ok(value.to_owned())
}

pub(super) fn is_ancestor(
    repository: &Path,
    quarantine: &Path,
    old: &str,
    new: &str,
) -> Result<bool, ReceiveHookError> {
    let status = git_status(
        repository,
        quarantine,
        ["merge-base", "--is-ancestor", old, new],
    )?;
    match status.code() {
        Some(0) => Ok(true),
        Some(1) => Ok(false),
        _ => Err(ReceiveHookError::GitCommand),
    }
}

pub(super) fn git_status<const N: usize>(
    repository: &Path,
    quarantine: &Path,
    arguments: [&str; N],
) -> Result<ExitStatus, ReceiveHookError> {
    git_command(repository, quarantine)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(ReceiveHookError::Io)
}

pub(super) fn git_output<const N: usize>(
    repository: &Path,
    quarantine: &Path,
    arguments: [&str; N],
) -> Result<Vec<u8>, ReceiveHookError> {
    let mut child = git_command(repository, quarantine)
        .args(arguments)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(ReceiveHookError::Io)?;
    let mut stdout = child.stdout.take().ok_or(ReceiveHookError::GitCommand)?;
    let mut output = Vec::new();
    stdout
        .by_ref()
        .take(MAX_INSPECTION_OUTPUT_BYTES + 1)
        .read_to_end(&mut output)
        .map_err(ReceiveHookError::Io)?;
    if u64::try_from(output.len()).unwrap_or(u64::MAX) > MAX_INSPECTION_OUTPUT_BYTES {
        let _ = child.kill();
        let _ = child.wait();
        return Err(ReceiveHookError::InspectionOutputLimitExceeded);
    }
    let status = child.wait().map_err(ReceiveHookError::Io)?;
    if !status.success() {
        return Err(ReceiveHookError::GitCommand);
    }
    Ok(output)
}

fn git_command(repository: &Path, quarantine: &Path) -> Command {
    let mut command = Command::new("git");
    command
        .arg("--git-dir")
        .arg(repository)
        .env_clear()
        .env("GIT_OBJECT_DIRECTORY", quarantine)
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            repository.join("objects"),
        );
    command
}

pub(super) fn canonical_repository(path: &Path) -> Result<PathBuf, ReceiveHookError> {
    let canonical = fs::canonicalize(path).map_err(ReceiveHookError::Io)?;
    let metadata = fs::symlink_metadata(path).map_err(ReceiveHookError::Io)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || !canonical.join("HEAD").is_file()
        || !canonical.join("objects").is_dir()
    {
        return Err(ReceiveHookError::InvalidRepositoryLayout);
    }
    Ok(canonical)
}

pub(super) fn canonical_quarantine(
    repository: &Path,
    path: &Path,
) -> Result<PathBuf, ReceiveHookError> {
    let canonical = fs::canonicalize(path).map_err(ReceiveHookError::Io)?;
    let objects = fs::canonicalize(repository.join("objects")).map_err(ReceiveHookError::Io)?;
    let metadata = fs::symlink_metadata(path).map_err(ReceiveHookError::Io)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || !canonical.starts_with(&objects)
        || canonical == objects
    {
        return Err(ReceiveHookError::InvalidQuarantine);
    }
    Ok(canonical)
}

pub(super) fn quarantine_stats(
    repository: &Path,
    quarantine: &Path,
    maximum_objects: u32,
) -> Result<(u64, u32), ReceiveHookError> {
    let mut bytes = 0_u64;
    let mut objects = 0_u32;
    let mut pending = vec![quarantine.to_owned()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory).map_err(ReceiveHookError::Io)? {
            let entry = entry.map_err(ReceiveHookError::Io)?;
            let file_type = entry.file_type().map_err(ReceiveHookError::Io)?;
            if file_type.is_symlink() {
                return Err(ReceiveHookError::InvalidQuarantine);
            }
            let metadata = entry.metadata().map_err(ReceiveHookError::Io)?;
            let path = entry.path();
            if metadata.is_dir() {
                pending.push(path);
                continue;
            }
            if !metadata.is_file() {
                return Err(ReceiveHookError::InvalidQuarantine);
            }
            if path.extension() == Some(OsStr::new("pack")) {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or(ReceiveHookError::TransferLimitExceeded)?;
            } else if path.extension() == Some(OsStr::new("idx")) {
                objects = objects
                    .checked_add(count_pack_objects(repository, quarantine, &path)?)
                    .ok_or(ReceiveHookError::TransferLimitExceeded)?;
            } else if is_loose_object(&path) {
                bytes = bytes
                    .checked_add(metadata.len())
                    .ok_or(ReceiveHookError::TransferLimitExceeded)?;
                objects = objects
                    .checked_add(1)
                    .ok_or(ReceiveHookError::TransferLimitExceeded)?;
            }
            if objects > maximum_objects {
                return Err(ReceiveHookError::TransferLimitExceeded);
            }
        }
    }
    Ok((bytes, objects))
}

fn count_pack_objects(
    repository: &Path,
    quarantine: &Path,
    index: &Path,
) -> Result<u32, ReceiveHookError> {
    let index = index.to_str().ok_or(ReceiveHookError::InvalidQuarantine)?;
    let output = git_output(repository, quarantine, ["verify-pack", "-v", index])?;
    output
        .split(|byte| *byte == b'\n')
        .try_fold(0_u32, |count, line| {
            let first = line.split(|byte| *byte == b' ').next().unwrap_or_default();
            if valid_hex_object_bytes(first) {
                count
                    .checked_add(1)
                    .ok_or(ReceiveHookError::TransferLimitExceeded)
            } else {
                Ok(count)
            }
        })
}

fn is_loose_object(path: &Path) -> bool {
    let Some(file) = path.file_name().and_then(OsStr::to_str) else {
        return false;
    };
    let Some(parent) = path
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
    else {
        return false;
    };
    parent.len() == 2
        && file.len() == 38
        && parent
            .bytes()
            .chain(file.bytes())
            .all(|byte| byte.is_ascii_hexdigit())
}

pub(super) fn parse_lines(bytes: &[u8]) -> Result<Vec<&str>, ReceiveHookError> {
    std::str::from_utf8(bytes)
        .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)
        .map(|text| text.lines().filter(|line| !line.is_empty()).collect())
}

pub(super) fn valid_object_name(value: &str) -> bool {
    valid_hex_object_bytes(value.as_bytes())
}

fn valid_hex_object_bytes(value: &[u8]) -> bool {
    matches!(value.len(), 40 | 64) && value.iter().all(u8::is_ascii_hexdigit)
}

pub(super) fn is_zero_object_name(value: &str) -> bool {
    value.bytes().all(|byte| byte == b'0')
}
