use super::{
    ReceiveHookError,
    commands::ReceiveCommand,
    repository::{
        canonical_ref, ensure_object_exists, git_output, is_ancestor, is_zero_object_name,
        parse_lines, peel_commit,
    },
};
use crate::receive_policy::{TrustedPathChange, TrustedReceiveUpdate};
use git_capability_domain::RefTransition;
use std::path::Path;

pub(super) fn inspect_update(
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
