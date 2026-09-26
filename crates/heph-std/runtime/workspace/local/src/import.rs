use crate::common::{
    Component, DeclaredFile, ImportCounters, ImportRequest, Imported, LocalWorkspaceConfig,
    LocalWorkspaceError, Manifest, Path, PathBuf, PermissionsExt, ensure_workspace_path, fs,
    git_output, git_text, import_directory, io_error, serialization, sha256,
    validate_relative_path, validate_repository,
};

pub fn import_result(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    sealed: &Path,
    request: &ImportRequest,
) -> Result<Imported, LocalWorkspaceError> {
    validate_repository(config, repository)?;
    ensure_workspace_path(config, sealed, "sealed")?;
    let work = sealed.join("work");
    let mut manifest = Manifest {
        version: 1,
        repository_id: Some(request.repository_id.to_string()),
        run_id: Some(request.run_id.to_string()),
        input_commit: Some(request.input_commit.clone()),
        result_tree: None,
        result_commit: None,
        entries: Vec::new(),
    };
    let mut counters = ImportCounters::default();
    let tree = import_directory(
        config,
        repository,
        &work,
        Path::new(""),
        &mut manifest,
        &mut counters,
    )?;
    let commit = git_text(
        config,
        repository,
        &["commit-tree", &tree, "-p", &request.input_commit],
        Some(request.message.as_bytes()),
        &[
            ("GIT_AUTHOR_NAME", "Hephaestus Agent"),
            ("GIT_AUTHOR_EMAIL", "agent@hephaestus.invalid"),
            ("GIT_COMMITTER_NAME", "Hephaestus Result Publisher"),
            ("GIT_COMMITTER_EMAIL", "result@hephaestus.invalid"),
            ("GIT_AUTHOR_DATE", &format!("@{} +0000", request.timestamp)),
            (
                "GIT_COMMITTER_DATE",
                &format!("@{} +0000", request.timestamp),
            ),
        ],
    )?;
    manifest.result_tree = Some(tree.clone());
    manifest.result_commit = Some(commit.clone());
    let manifest_bytes = serde_json::to_vec(&manifest).map_err(serialization)?;
    let manifest_hash = sha256(&manifest_bytes);
    let patch = git_output(
        config,
        repository,
        &[
            "diff-tree",
            "-p",
            "--binary",
            "--no-ext-diff",
            &request.input_commit,
            &commit,
        ],
        None,
        &[],
    )?;
    if patch.len() > config.limits.max_patch_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "generated result patch exceeds configured limit",
        )));
    }
    let mut declared_files = Vec::with_capacity(request.declared_paths.len());
    for declared_path in &request.declared_paths {
        let (declared_host_path, metadata) = declared_regular_file(&work, declared_path)?;
        let bytes = fs::read(&declared_host_path).map_err(io_error)?;
        declared_files.push(DeclaredFile {
            path: declared_path.clone(),
            mode: if metadata.permissions().mode() & 0o111 != 0 {
                0o100_755
            } else {
                0o100_644
            },
            sha256: sha256(&bytes),
            bytes,
        });
    }
    Ok(Imported {
        tree,
        commit,
        manifest: manifest_bytes,
        manifest_hash,
        patch,
        declared_files,
    })
}

pub fn declared_regular_file(
    work: &Path,
    declared_path: &str,
) -> Result<(PathBuf, fs::Metadata), LocalWorkspaceError> {
    validate_relative_path(declared_path)?;
    let components = Path::new(declared_path).components().collect::<Vec<_>>();
    let mut current = work.to_owned();
    for (index, component) in components.iter().enumerate() {
        let Component::Normal(component) = component else {
            return Err(LocalWorkspaceError::UnsafePath(format!(
                "unsafe declared result path {declared_path:?}"
            )));
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            LocalWorkspaceError::InvalidResult(format!(
                "declared result file {declared_path:?} cannot be inspected: {error}"
            ))
        })?;
        let is_final = index + 1 == components.len();
        if is_final && metadata.file_type().is_file() {
            return Ok((current, metadata));
        }
        if !is_final && metadata.file_type().is_dir() {
            continue;
        }
        return Err(LocalWorkspaceError::InvalidResult(format!(
            "declared result path {declared_path:?} contains a symlink or non-directory parent"
        )));
    }
    Err(LocalWorkspaceError::InvalidResult(format!(
        "declared result path {declared_path:?} is not a regular file"
    )))
}
