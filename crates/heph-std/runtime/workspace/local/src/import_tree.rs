use crate::common::{
    ImportCounters, LocalWorkspaceConfig, LocalWorkspaceError, Manifest, ManifestEntry, OsStrExt,
    Path, PermissionsExt, TreeObject, Write, enforce_bytes, fs, git_text, io_error, sha256,
    utf8_path, validate_name, validate_symlink_target,
};

// Recursive import keeps the checks adjacent to each supported file kind.
#[allow(clippy::too_many_lines)]
pub fn import_directory(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    directory: &Path,
    relative: &Path,
    manifest: &mut Manifest,
    counters: &mut ImportCounters,
) -> Result<String, LocalWorkspaceError> {
    let mut children = fs::read_dir(directory)
        .map_err(io_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(io_error)?;
    children.sort_by_key(std::fs::DirEntry::file_name);
    let mut tree_entries = Vec::with_capacity(children.len());
    for child in children {
        counters.entries += 1;
        if counters.entries > config.limits.max_entries {
            return Err(LocalWorkspaceError::Quota(String::from(
                "result tree exceeds entry limit",
            )));
        }
        let name = child.file_name().into_string().map_err(|_| {
            LocalWorkspaceError::InvalidResult(String::from("result path is not UTF-8"))
        })?;
        if name.eq_ignore_ascii_case(".git") {
            return Err(LocalWorkspaceError::InvalidResult(String::from(
                ".git paths are forbidden in result workspaces",
            )));
        }
        validate_name(&name)?;
        let child_relative = relative.join(&name);
        let path = child.path();
        let metadata = fs::symlink_metadata(&path).map_err(io_error)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            let object_id = import_directory(
                config,
                repository,
                &path,
                &child_relative,
                manifest,
                counters,
            )?;
            tree_entries.push(TreeObject {
                mode: 0o040_000,
                kind: "tree",
                object_id,
                name,
            });
        } else if file_type.is_file() {
            let size = metadata.len();
            enforce_bytes(config, &mut counters.bytes, size)?;
            let bytes = fs::read(&path).map_err(io_error)?;
            let executable = metadata.permissions().mode() & 0o111 != 0;
            let mode = if executable { 0o100_755 } else { 0o100_644 };
            let object_id = git_text(
                config,
                repository,
                &["hash-object", "-w", "--stdin"],
                Some(&bytes),
                &[],
            )?;
            manifest.entries.push(ManifestEntry {
                path: utf8_path(&child_relative)?,
                kind: "file",
                mode,
                size,
                sha256: sha256(&bytes),
            });
            tree_entries.push(TreeObject {
                mode,
                kind: "blob",
                object_id,
                name,
            });
        } else if file_type.is_symlink() {
            let target = fs::read_link(&path).map_err(io_error)?;
            let target_bytes = target.as_os_str().as_bytes();
            enforce_bytes(config, &mut counters.bytes, target_bytes.len() as u64)?;
            let target_text = std::str::from_utf8(target_bytes).map_err(|_| {
                LocalWorkspaceError::InvalidResult(String::from("symlink target is not UTF-8"))
            })?;
            validate_symlink_target(target_text)?;
            let object_id = git_text(
                config,
                repository,
                &["hash-object", "-w", "--stdin"],
                Some(target_bytes),
                &[],
            )?;
            manifest.entries.push(ManifestEntry {
                path: utf8_path(&child_relative)?,
                kind: "symlink",
                mode: 0o120_000,
                size: target_bytes.len() as u64,
                sha256: sha256(target_bytes),
            });
            tree_entries.push(TreeObject {
                mode: 0o120_000,
                kind: "blob",
                object_id,
                name,
            });
        } else {
            return Err(LocalWorkspaceError::InvalidResult(format!(
                "unsupported filesystem object at {}",
                child_relative.display()
            )));
        }
    }
    let mut input = Vec::new();
    for entry in tree_entries {
        write!(
            input,
            "{:06o} {} {}\t{}\0",
            entry.mode, entry.kind, entry.object_id, entry.name
        )
        .map_err(io_error)?;
    }
    git_text(config, repository, &["mktree", "-z"], Some(&input), &[])
}
