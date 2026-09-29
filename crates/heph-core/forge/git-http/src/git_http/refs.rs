use crate::{GitHttpError, errors::domain};
use forge_domain::{CommitSha, GitRef, RefUpdate};
use std::{collections::BTreeMap, path::PathBuf, process::Stdio};
use tokio::process::Command;

pub async fn snapshot_refs(path: PathBuf) -> Result<BTreeMap<GitRef, CommitSha>, GitHttpError> {
    let output = Command::new("git")
        .arg("--git-dir")
        .arg(path)
        .arg("for-each-ref")
        .arg("--format=%(refname) %(objectname)")
        .env_clear()
        .stdin(Stdio::null())
        .output()
        .await
        .map_err(GitHttpError::Io)?;
    if !output.status.success() {
        return Err(GitHttpError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| GitHttpError::Git(String::from("ref output was not UTF-8")))?;
    text.lines()
        .map(|line| {
            let (name, commit) = line
                .split_once(' ')
                .ok_or_else(|| GitHttpError::Git(String::from("malformed ref output")))?;
            Ok((
                GitRef::parse(name.to_owned()).map_err(domain)?,
                CommitSha::parse(commit.to_owned()).map_err(domain)?,
            ))
        })
        .collect()
}

pub fn diff_refs(
    before: &BTreeMap<GitRef, CommitSha>,
    after: &BTreeMap<GitRef, CommitSha>,
) -> Vec<RefUpdate> {
    let mut names = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<Vec<_>>();
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter_map(|git_ref| {
            let old_commit = before.get(&git_ref).cloned();
            let new_commit = after.get(&git_ref).cloned();
            (old_commit != new_commit).then_some(RefUpdate {
                git_ref,
                old_commit,
                new_commit,
            })
        })
        .collect()
}
