use std::path::Path;

use super::{
    BrowserError, DiffFile, DiffFileState, MAX_CHANGED_FILES, MAX_DIFF_BYTES_PER_FILE,
    diff_parse::parse_hunks,
    git::{git_owned, text},
};

pub(super) async fn changed_files(
    repository: &Path,
    commit: &str,
    parent: &str,
) -> Result<(Vec<DiffFile>, bool), BrowserError> {
    let args = if parent.is_empty() {
        vec![
            String::from("diff-tree"),
            String::from("--root"),
            String::from("--no-commit-id"),
            String::from("--find-renames"),
            String::from("-r"),
            String::from("--name-status"),
            String::from("-z"),
            commit.to_owned(),
        ]
    } else {
        vec![
            String::from("diff"),
            String::from("--find-renames"),
            String::from("--name-status"),
            String::from("-z"),
            parent.to_owned(),
            commit.to_owned(),
        ]
    };
    let output = git_owned(repository, &args, 1_048_576).await?;
    let mut values = Vec::new();
    let mut fields = output.split(|byte| *byte == 0);
    loop {
        let Some(status) = fields.next() else {
            break;
        };
        if status.is_empty() {
            continue;
        }
        let status = text(status)?;
        let first_path = text(fields.next().ok_or(BrowserError::Git)?)?;
        let (path, previous_path, state) = match status.as_bytes().first() {
            Some(b'A') => (first_path, String::new(), DiffFileState::Added),
            Some(b'D') => (first_path, String::new(), DiffFileState::Deleted),
            Some(b'M' | b'T' | b'C') => (first_path, String::new(), DiffFileState::Modified),
            Some(b'R') => (
                text(fields.next().ok_or(BrowserError::Git)?)?,
                first_path,
                DiffFileState::Renamed,
            ),
            _ => return Err(BrowserError::Git),
        };
        if values.len() == MAX_CHANGED_FILES {
            return Ok((values, true));
        }
        values.push(DiffFile {
            path,
            previous_path,
            state,
            additions: 0,
            deletions: 0,
            hunks: Vec::new(),
            truncated: false,
        });
    }
    Ok((values, false))
}

pub(super) async fn populate_hunks(
    repository: &Path,
    commit: &str,
    parent: &str,
    file: &mut DiffFile,
) -> Result<(), BrowserError> {
    let args = if parent.is_empty() {
        vec![
            String::from("diff-tree"),
            String::from("--root"),
            String::from("--no-commit-id"),
            String::from("--no-ext-diff"),
            String::from("--no-color"),
            String::from("--unified=3"),
            commit.to_owned(),
            String::from("--"),
            file.path.clone(),
        ]
    } else {
        vec![
            String::from("diff"),
            String::from("--no-ext-diff"),
            String::from("--no-color"),
            String::from("--unified=3"),
            parent.to_owned(),
            commit.to_owned(),
            String::from("--"),
            file.path.clone(),
        ]
    };
    let output = match git_owned(repository, &args, MAX_DIFF_BYTES_PER_FILE).await {
        Ok(output) => output,
        Err(BrowserError::ResourceExhausted) => {
            file.state = DiffFileState::Truncated;
            file.truncated = true;
            return Ok(());
        }
        // One malformed or temporarily unavailable object must not turn an
        // otherwise authorized, bounded commit inspection into an error page.
        Err(BrowserError::Git | BrowserError::Storage(_)) => {
            file.state = DiffFileState::Unavailable;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    let Ok(source) = String::from_utf8(output) else {
        file.state = DiffFileState::Binary;
        return Ok(());
    };
    if source.contains("Binary files ") || source.contains("GIT binary patch") {
        file.state = DiffFileState::Binary;
        return Ok(());
    }
    let (hunks, additions, deletions, truncated) = match parse_hunks(&source) {
        Ok(parsed) => parsed,
        Err(BrowserError::Git) => {
            file.state = DiffFileState::Unavailable;
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    file.hunks = hunks;
    file.additions = additions;
    file.deletions = deletions;
    file.truncated = truncated;
    if truncated {
        file.state = DiffFileState::Truncated;
    }
    Ok(())
}
