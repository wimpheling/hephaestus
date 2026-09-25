use crate::common::{LocalWorkspaceConfig, LocalWorkspaceError, Path, git_output};

pub struct GitTreeEntry {
    pub mode: u32,
    pub kind: String,
    pub object_id: String,
    pub path: String,
}

pub fn git_ls_tree(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    commit: &str,
) -> Result<Vec<GitTreeEntry>, LocalWorkspaceError> {
    let output = git_output(
        config,
        repository,
        &["ls-tree", "-rz", "-r", "--full-tree", commit],
        None,
        &[],
    )?;
    let mut entries = Vec::new();
    for record in output
        .split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
    {
        let tab = record
            .iter()
            .position(|byte| *byte == b'\t')
            .ok_or_else(|| {
                LocalWorkspaceError::Git(String::from("ls-tree record has no path separator"))
            })?;
        let metadata = std::str::from_utf8(&record[..tab])
            .map_err(|_| LocalWorkspaceError::Git(String::from("ls-tree metadata is not UTF-8")))?;
        let path = std::str::from_utf8(&record[tab + 1..])
            .map_err(|_| LocalWorkspaceError::InvalidSource(String::from("non-UTF-8 Git path")))?;
        let mut fields = metadata.split_ascii_whitespace();
        let mode = u32::from_str_radix(
            fields.next().ok_or_else(|| {
                LocalWorkspaceError::Git(String::from("ls-tree record has no mode"))
            })?,
            8,
        )
        .map_err(|error| LocalWorkspaceError::Git(error.to_string()))?;
        let kind = fields
            .next()
            .ok_or_else(|| LocalWorkspaceError::Git(String::from("ls-tree record has no type")))?;
        let object_id = fields.next().ok_or_else(|| {
            LocalWorkspaceError::Git(String::from("ls-tree record has no object ID"))
        })?;
        entries.push(GitTreeEntry {
            mode,
            kind: kind.to_owned(),
            object_id: object_id.to_owned(),
            path: path.to_owned(),
        });
    }
    if entries.len() > config.limits.max_entries {
        return Err(LocalWorkspaceError::Quota(String::from(
            "source tree exceeds entry limit",
        )));
    }
    Ok(entries)
}
