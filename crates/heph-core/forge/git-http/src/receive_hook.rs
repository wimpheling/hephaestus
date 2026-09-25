//! Trusted inspection for Git's quarantined `pre-receive` boundary.
//!
//! Git invokes `pre-receive` after unpacking incoming objects into quarantine
//! and before its atomic ref transaction. This adapter treats hook commands as
//! proposals only: it verifies every old object against canonical refs and
//! derives transitions and paths from Git's object database before applying
//! the capability grammar.

use crate::receive_policy::{
    CapabilityReceivePolicyGuard, ReceivePolicyError, ReceivePolicyGuard,
    ResolvedRuntimeReceiveContext, TrustedPathChange, TrustedReceiveProposal, TrustedReceiveUpdate,
};
use git_capability_domain::{RefTransition, RepositoryId};
use std::{
    ffi::OsStr,
    fs,
    io::{self, BufRead, Read},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
};

const MAX_INSPECTION_OUTPUT_BYTES: u64 = 64 * 1_024 * 1_024;

/// Host-owned inputs for one quarantined receive inspection.
#[derive(Debug)]
pub struct ReceiveHookInput<'a> {
    /// Canonical bare repository selected by the trusted HTTP route.
    pub repository_path: &'a Path,
    /// Repository identifier selected by the trusted HTTP route.
    pub repository_id: RepositoryId,
    /// Quarantine created by `git-receive-pack` for this transaction.
    pub quarantine_path: &'a Path,
    /// Actual HTTP bytes, already bounded by the host transport.
    pub request_bytes: u64,
    /// Current host wall-clock time used for the final expiry check.
    pub now_unix_seconds: i64,
}

/// Inspects and authorizes a complete `pre-receive` command stream.
///
/// Success means the exact proposal may proceed to Git's canonical atomic ref
/// transaction. Failure must be returned as a non-zero hook exit status.
///
/// # Errors
///
/// Returns a fail-closed error if repository facts cannot be derived, the
/// transaction exceeds its immutable bounds, or capability policy denies any
/// update.
pub fn authorize_quarantined_receive(
    context: ResolvedRuntimeReceiveContext,
    input: &ReceiveHookInput<'_>,
    commands: impl BufRead,
) -> Result<TrustedReceiveProposal, ReceiveHookError> {
    if context.repository_id() != input.repository_id
        || !context.is_active_at(input.now_unix_seconds)
    {
        return Err(ReceiveHookError::ContextMismatch);
    }
    let repository = canonical_repository(input.repository_path)?;
    let quarantine = canonical_quarantine(&repository, input.quarantine_path)?;
    let limits = context.transfer_limits();
    if input.request_bytes > limits.request_bytes() {
        return Err(ReceiveHookError::TransferLimitExceeded);
    }
    let (pack_bytes, object_count) =
        quarantine_stats(&repository, &quarantine, limits.object_count())?;
    if pack_bytes > limits.pack_bytes() || object_count > limits.object_count() {
        return Err(ReceiveHookError::TransferLimitExceeded);
    }
    let commands = parse_commands(commands, limits.ref_updates())?;
    let mut updates = Vec::with_capacity(commands.len());
    for command in commands {
        updates.push(inspect_update(&repository, &quarantine, command)?);
    }
    let proposal = TrustedReceiveProposal::new(
        context,
        updates,
        input.request_bytes,
        pack_bytes,
        object_count,
    );
    CapabilityReceivePolicyGuard
        .authorize(&proposal)
        .map_err(ReceiveHookError::Policy)?;
    Ok(proposal)
}

#[derive(Debug)]
struct ReceiveCommand {
    old: String,
    new: String,
    reference: String,
}

fn parse_commands(
    commands: impl BufRead,
    maximum: u16,
) -> Result<Vec<ReceiveCommand>, ReceiveHookError> {
    let mut parsed = Vec::new();
    for line in commands.lines().take(usize::from(maximum) + 1) {
        if parsed.len() == usize::from(maximum) {
            return Err(ReceiveHookError::InvalidCommandBatch);
        }
        let line = line.map_err(ReceiveHookError::Io)?;
        let mut fields = line.split(' ');
        let (Some(old), Some(new), Some(reference), None) =
            (fields.next(), fields.next(), fields.next(), fields.next())
        else {
            return Err(ReceiveHookError::InvalidCommandBatch);
        };
        if !valid_object_name(old)
            || !valid_object_name(new)
            || old.len() != new.len()
            || !reference.starts_with("refs/")
            || reference.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(ReceiveHookError::InvalidCommandBatch);
        }
        parsed.push(ReceiveCommand {
            old: old.to_owned(),
            new: new.to_owned(),
            reference: reference.to_owned(),
        });
    }
    if parsed.is_empty() {
        return Err(ReceiveHookError::InvalidCommandBatch);
    }
    Ok(parsed)
}

fn inspect_update(
    repository: &Path,
    quarantine: &Path,
    command: ReceiveCommand,
) -> Result<TrustedReceiveUpdate, ReceiveHookError> {
    let old_is_zero = is_zero_object_name(&command.old);
    let new_is_zero = is_zero_object_name(&command.new);
    let canonical_old = canonical_ref(repository, &command.reference)?;
    match (old_is_zero, canonical_old.as_deref()) {
        (true, None) => {}
        (false, Some(actual)) if actual == command.old => {}
        _ => return Err(ReceiveHookError::StaleOrForgedCommand),
    }

    let (transition, changed_paths) = if new_is_zero {
        if old_is_zero {
            return Err(ReceiveHookError::InvalidCommandBatch);
        }
        (RefTransition::Delete, Vec::new())
    } else {
        ensure_object_exists(repository, quarantine, &command.new)?;
        if old_is_zero {
            let new_commit = peel_commit(repository, quarantine, &command.new)?;
            (
                RefTransition::Create,
                diff_paths(repository, quarantine, EMPTY_TREE, &new_commit)?,
            )
        } else {
            let old_commit = peel_commit(repository, quarantine, &command.old)?;
            let new_commit = peel_commit(repository, quarantine, &command.new)?;
            let fast_forward = is_ancestor(repository, quarantine, &old_commit, &new_commit)?;
            let mut paths = diff_paths_owned(repository, quarantine, &old_commit, &new_commit)?;
            paths.extend(merge_parent_paths(
                repository,
                quarantine,
                &old_commit,
                &new_commit,
            )?);
            paths.sort();
            paths.dedup();
            (
                RefTransition::Update { fast_forward },
                paths
                    .into_iter()
                    .map(OwnedPathChange::into_trusted)
                    .collect(),
            )
        }
    };
    Ok(TrustedReceiveUpdate::new_with_old_object(
        command.reference,
        transition,
        changed_paths,
        (!old_is_zero).then_some(command.old),
    ))
}

const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

fn merge_parent_paths(
    repository: &Path,
    quarantine: &Path,
    old: &str,
    new: &str,
) -> Result<Vec<OwnedPathChange>, ReceiveHookError> {
    let range = format!("{old}..{new}");
    let output = git_output(repository, quarantine, ["rev-list", "--merges", &range])?;
    let commits = parse_lines(&output)?;
    let mut paths = Vec::new();
    for commit in commits {
        let parents_output = git_output(
            repository,
            quarantine,
            ["rev-list", "--parents", "-n", "1", commit],
        )?;
        let parents = std::str::from_utf8(&parents_output)
            .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)?
            .trim()
            .split(' ')
            .skip(1)
            .collect::<Vec<_>>();
        if parents.len() < 2 {
            return Err(ReceiveHookError::InvalidRepositoryFacts);
        }
        for parent in parents {
            paths.extend(diff_paths_owned(repository, quarantine, parent, commit)?);
        }
    }
    Ok(paths)
}

fn diff_paths(
    repository: &Path,
    quarantine: &Path,
    old: &str,
    new: &str,
) -> Result<Vec<TrustedPathChange>, ReceiveHookError> {
    Ok(diff_paths_owned(repository, quarantine, old, new)?
        .into_iter()
        .map(OwnedPathChange::into_trusted)
        .collect())
}

fn diff_paths_owned(
    repository: &Path,
    quarantine: &Path,
    old: &str,
    new: &str,
) -> Result<Vec<OwnedPathChange>, ReceiveHookError> {
    let output = git_output(
        repository,
        quarantine,
        [
            "-c",
            "diff.renames=true",
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            "-z",
            "-M",
            "-C",
            old,
            new,
        ],
    )?;
    parse_name_status(&output)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum OwnedPathChange {
    Addition(String),
    Modification(String),
    Deletion(String),
    Rename { from: String, to: String },
}

impl OwnedPathChange {
    fn into_trusted(self) -> TrustedPathChange {
        match self {
            Self::Addition(path) => TrustedPathChange::Addition(path),
            Self::Modification(path) => TrustedPathChange::Modification(path),
            Self::Deletion(path) => TrustedPathChange::Deletion(path),
            Self::Rename { from, to } => TrustedPathChange::Rename { from, to },
        }
    }
}

fn parse_name_status(bytes: &[u8]) -> Result<Vec<OwnedPathChange>, ReceiveHookError> {
    let fields = bytes
        .split(|byte| *byte == 0)
        .filter(|field| !field.is_empty())
        .collect::<Vec<_>>();
    let mut changes = Vec::new();
    let mut index = 0;
    while index < fields.len() {
        let status = std::str::from_utf8(fields[index])
            .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)?;
        index += 1;
        let path = fields
            .get(index)
            .ok_or(ReceiveHookError::InvalidRepositoryFacts)
            .and_then(|value| {
                std::str::from_utf8(value).map_err(|_| ReceiveHookError::InvalidRepositoryFacts)
            })?;
        index += 1;
        let change = match status.as_bytes().first().copied() {
            Some(b'A') => OwnedPathChange::Addition(path.to_owned()),
            Some(b'D') => OwnedPathChange::Deletion(path.to_owned()),
            Some(b'M' | b'T') => OwnedPathChange::Modification(path.to_owned()),
            Some(b'R' | b'C') => {
                let to = fields
                    .get(index)
                    .ok_or(ReceiveHookError::InvalidRepositoryFacts)
                    .and_then(|value| {
                        std::str::from_utf8(value)
                            .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)
                    })?;
                index += 1;
                OwnedPathChange::Rename {
                    from: path.to_owned(),
                    to: to.to_owned(),
                }
            }
            _ => return Err(ReceiveHookError::InvalidRepositoryFacts),
        };
        changes.push(change);
    }
    Ok(changes)
}

fn canonical_ref(repository: &Path, reference: &str) -> Result<Option<String>, ReceiveHookError> {
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

fn ensure_object_exists(
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

fn peel_commit(
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

fn is_ancestor(
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

fn git_status<const N: usize>(
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

fn git_output<const N: usize>(
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

fn canonical_repository(path: &Path) -> Result<PathBuf, ReceiveHookError> {
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

fn canonical_quarantine(repository: &Path, path: &Path) -> Result<PathBuf, ReceiveHookError> {
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

fn quarantine_stats(
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

fn parse_lines(bytes: &[u8]) -> Result<Vec<&str>, ReceiveHookError> {
    std::str::from_utf8(bytes)
        .map_err(|_| ReceiveHookError::InvalidRepositoryFacts)
        .map(|text| text.lines().filter(|line| !line.is_empty()).collect())
}

fn valid_object_name(value: &str) -> bool {
    valid_hex_object_bytes(value.as_bytes())
}

fn valid_hex_object_bytes(value: &[u8]) -> bool {
    matches!(value.len(), 40 | 64) && value.iter().all(u8::is_ascii_hexdigit)
}

fn is_zero_object_name(value: &str) -> bool {
    value.bytes().all(|byte| byte == b'0')
}

/// Fail-closed pre-receive inspection error.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReceiveHookError {
    /// Host authority does not match the selected repository or current time.
    #[error("runtime receive context does not match this transaction")]
    ContextMismatch,
    /// The hook command stream was empty, malformed, or over its bound.
    #[error("runtime receive command batch is invalid")]
    InvalidCommandBatch,
    /// A client-declared old object did not match the canonical ref.
    #[error("runtime receive command is stale or inconsistent with canonical state")]
    StaleOrForgedCommand,
    /// Canonical bare storage was not trustworthy.
    #[error("runtime receive repository layout is invalid")]
    InvalidRepositoryLayout,
    /// Git's quarantine was absent or outside canonical object storage.
    #[error("runtime receive quarantine is invalid")]
    InvalidQuarantine,
    /// Trusted object or ancestry facts could not be derived.
    #[error("runtime receive repository facts are invalid")]
    InvalidRepositoryFacts,
    /// A bounded Git inspection subprocess failed.
    #[error("runtime receive repository inspection failed")]
    GitCommand,
    /// Inspection output exceeded its host-side safety ceiling.
    #[error("runtime receive inspection output exceeded its safety limit")]
    InspectionOutputLimitExceeded,
    /// Object or byte ceilings were exceeded.
    #[error("runtime receive transfer limit was exceeded")]
    TransferLimitExceeded,
    /// Capability policy denied the trusted proposal.
    #[error(transparent)]
    Policy(#[from] ReceivePolicyError),
    /// Local storage inspection failed.
    #[error("runtime receive local inspection failed")]
    Io(#[source] io::Error),
}
