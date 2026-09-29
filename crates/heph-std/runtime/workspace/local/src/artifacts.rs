use crate::common::{
    ArtifactId, Imported, LocalWorkspaceConfig, LocalWorkspaceError, Path, ResultArtifactMetadata,
    RunId, fs, git_optional_text, git_output, integer_error, io_error, serialization, sha256,
    sync_directory, utf8_path, write_new_file,
};

#[allow(clippy::too_many_lines)] // Artifact metadata mirrors the durable result schema explicitly.
pub fn persist_artifacts(
    imported: &Imported,
    logs: &[serde_json::Value],
    exit: &serde_json::Value,
    run_id: RunId,
    artifact_root: &Path,
) -> Result<Vec<ResultArtifactMetadata>, LocalWorkspaceError> {
    let logs = serde_json::to_vec(logs).map_err(serialization)?;
    let exit = serde_json::to_vec(exit).map_err(serialization)?;
    let artifact_directory = artifact_root.join(run_id.to_string());
    fs::create_dir_all(&artifact_directory).map_err(io_error)?;
    let manifest_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "manifest",
        &imported.manifest_hash,
        "json",
        &imported.manifest,
    )?;
    let patch_hash = sha256(&imported.patch);
    let patch_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "patch",
        &patch_hash,
        "patch",
        &imported.patch,
    )?;
    let logs_hash = sha256(&logs);
    let logs_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "logs",
        &logs_hash,
        "json",
        &logs,
    )?;
    let exit_hash = sha256(&exit);
    let exit_key = store_artifact(
        artifact_root,
        &artifact_directory,
        "exit",
        &exit_hash,
        "json",
        &exit,
    )?;
    let mut artifacts = vec![
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("manifest"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(imported.manifest.len()).map_err(integer_error)?,
            sha256: imported.manifest_hash.clone(),
            storage_key: manifest_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("logs"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(logs.len()).map_err(integer_error)?,
            sha256: logs_hash,
            storage_key: logs_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("exit"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("application/json"),
            size_bytes: i64::try_from(exit.len()).map_err(integer_error)?,
            sha256: exit_hash,
            storage_key: exit_key,
        },
        ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("patch"),
            path: String::new(),
            git_mode: None,
            media_type: String::from("text/x-diff"),
            size_bytes: i64::try_from(imported.patch.len()).map_err(integer_error)?,
            sha256: patch_hash,
            storage_key: patch_key,
        },
    ];
    for declared in &imported.declared_files {
        let key = store_artifact(
            artifact_root,
            &artifact_directory,
            "declared",
            &declared.sha256,
            "bin",
            &declared.bytes,
        )?;
        artifacts.push(ResultArtifactMetadata {
            id: ArtifactId::new().as_uuid(),
            kind: String::from("declared_file"),
            path: declared.path.clone(),
            git_mode: Some(i32::try_from(declared.mode).map_err(integer_error)?),
            media_type: String::from("application/octet-stream"),
            size_bytes: i64::try_from(declared.bytes.len()).map_err(integer_error)?,
            sha256: declared.sha256.clone(),
            storage_key: key,
        });
    }
    Ok(artifacts)
}

pub fn store_artifact(
    artifact_root: &Path,
    directory: &Path,
    kind: &str,
    hash: &str,
    extension: &str,
    bytes: &[u8],
) -> Result<String, LocalWorkspaceError> {
    let name = format!("{kind}-{hash}.{extension}");
    let destination = directory.join(&name);
    if !destination.exists() {
        let temporary = directory.join(format!(".{name}.tmp"));
        write_new_file(&temporary, bytes, false)?;
        fs::rename(&temporary, &destination).map_err(io_error)?;
        sync_directory(directory)?;
    }
    let relative = destination.strip_prefix(artifact_root).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("artifact escaped artifact root"))
    })?;
    utf8_path(relative)
}

pub fn cas_publish_ref(
    config: &LocalWorkspaceConfig,
    repository: &Path,
    result_ref: &str,
    commit: &str,
) -> Result<(), LocalWorkspaceError> {
    let existing = git_optional_text(config, repository, &["rev-parse", "--verify", result_ref])?;
    if let Some(existing) = existing {
        if existing == commit {
            return Ok(());
        }
        return Err(LocalWorkspaceError::Integrity(format!(
            "result ref {result_ref} points to {existing}, expected {commit}"
        )));
    }
    let zero = "0".repeat(commit.len());
    git_output(
        config,
        repository,
        &["update-ref", result_ref, commit, &zero],
        None,
        &[],
    )?;
    Ok(())
}
