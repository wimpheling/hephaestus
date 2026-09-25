use crate::common::{
    LocalWorkspaceConfig, LocalWorkspaceError, Manifest, ManifestEntry, Materialized, OsStr, Path,
    copy_tree, enforce_bytes, ensure_workspace_path, fs, fsync_tree, git_ls_tree, git_output,
    git_text, io_error, make_source_read_only, serialization, sha256, symlink, sync_directory,
    validate_relative_path, validate_repository, validate_symlink_target, write_new_file,
    write_owner_marker,
};

pub fn materialize(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    commit: &str,
    temporary: &Path,
    active: &Path,
) -> Result<Materialized, LocalWorkspaceError> {
    validate_repository(config, repository)?;
    ensure_workspace_path(config, temporary, "active")?;
    ensure_workspace_path(config, active, "active")?;
    if temporary.exists() || active.exists() {
        return Err(LocalWorkspaceError::State(String::from(
            "workspace path already exists",
        )));
    }
    fs::create_dir(temporary).map_err(io_error)?;
    let owner = active
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("active owner is invalid")))?;
    write_owner_marker(temporary, owner)?;
    let source = temporary.join("source");
    let work = temporary.join("work");
    fs::create_dir(&source).map_err(io_error)?;
    let entries = git_ls_tree(config, repository, commit)?;
    let mut manifest = Manifest {
        version: 1,
        repository_id: None,
        run_id: None,
        input_commit: None,
        result_tree: None,
        result_commit: None,
        entries: Vec::with_capacity(entries.len()),
    };
    let mut total = 0_u64;
    for entry in entries {
        validate_relative_path(&entry.path)?;
        if entry.kind != "blob" || !matches!(entry.mode, 0o100_644 | 0o100_755 | 0o120_000) {
            return Err(LocalWorkspaceError::InvalidSource(format!(
                "unsupported Git entry {} {:o} {}",
                entry.kind, entry.mode, entry.path
            )));
        }
        let bytes = git_output(
            config,
            repository,
            &["cat-file", "blob", &entry.object_id],
            None,
            &[],
        )?;
        enforce_bytes(config, &mut total, bytes.len() as u64)?;
        let destination = source.join(&entry.path);
        let parent = destination.parent().ok_or_else(|| {
            LocalWorkspaceError::UnsafePath(String::from("source entry has no parent"))
        })?;
        fs::create_dir_all(parent).map_err(io_error)?;
        if entry.mode == 0o120_000 {
            let target = std::str::from_utf8(&bytes).map_err(|_| {
                LocalWorkspaceError::InvalidSource(format!(
                    "symlink target for {} is not UTF-8",
                    entry.path
                ))
            })?;
            validate_symlink_target(target)?;
            symlink(target, &destination).map_err(io_error)?;
        } else {
            write_new_file(&destination, &bytes, entry.mode & 0o111 != 0)?;
        }
        manifest.entries.push(ManifestEntry {
            path: entry.path,
            kind: if entry.mode == 0o120_000 {
                "symlink"
            } else {
                "file"
            },
            mode: entry.mode,
            size: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
    }
    copy_tree(&source, &work)?;
    make_source_read_only(&source)?;
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(serialization)?;
    let manifest_hash = sha256(&manifest_bytes);
    fsync_tree(temporary)?;
    fs::rename(temporary, active).map_err(io_error)?;
    sync_directory(
        active
            .parent()
            .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("active root missing")))?,
    )?;
    let mut treeish = commit.to_owned();
    treeish.push('^');
    treeish.push('{');
    treeish.push_str("tree");
    treeish.push('}');
    let tree = git_text(config, repository, &["rev-parse", &treeish], None, &[])?;
    Ok(Materialized {
        tree,
        manifest_hash,
    })
}
