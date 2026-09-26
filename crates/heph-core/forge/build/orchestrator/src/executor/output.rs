use super::{
    ArtifactKind, BuildArtifactKind, BuildConfig, BuildExecutionError, BuildRequestId, Deserialize,
    ImportedArtifact, Path, ReleaseArtifactId, ReleaseArtifactInput, Uuid, Value, filesystem,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::os::unix::fs::MetadataExt;
use std::{fs, os::unix::fs::PermissionsExt};

pub(super) fn seal_output(root: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(BuildExecutionError::UnsafeOutput);
    }
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            seal_output(&entry.path())?;
        } else if metadata.file_type().is_file() && metadata.nlink() == 1 {
            let executable = metadata.permissions().mode() & 0o111 != 0;
            fs::set_permissions(
                entry.path(),
                fs::Permissions::from_mode(if executable { 0o500 } else { 0o400 }),
            )
            .map_err(filesystem)?;
        } else {
            return Err(BuildExecutionError::UnsafeOutput);
        }
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o500)).map_err(filesystem)
}

pub(super) fn validate_declared_outputs(
    build: &BuildConfig,
    imported: &[ImportedArtifact],
) -> Result<(), BuildExecutionError> {
    for artifact in imported {
        if !build.artifacts.iter().any(|declared| {
            let path = artifact.path.as_str();
            match declared.kind {
                BuildArtifactKind::Directory => path
                    .strip_prefix(&declared.path)
                    .is_some_and(|suffix| suffix.starts_with('/')),
                BuildArtifactKind::File => {
                    path == declared.path && artifact.kind == ArtifactKind::File
                }
                BuildArtifactKind::Executable => {
                    path == declared.path && artifact.kind == ArtifactKind::Executable
                }
            }
        }) {
            return Err(BuildExecutionError::UndeclaredOutput);
        }
    }
    for declared in &build.artifacts {
        let present = imported.iter().any(|artifact| match declared.kind {
            BuildArtifactKind::Directory => artifact
                .path
                .as_str()
                .strip_prefix(&declared.path)
                .is_some_and(|suffix| suffix.starts_with('/')),
            BuildArtifactKind::File | BuildArtifactKind::Executable => {
                artifact.path.as_str() == declared.path
            }
        });
        if !present {
            return Err(BuildExecutionError::MissingOutput);
        }
    }
    Ok(())
}

pub(super) fn release_inputs(
    build_request_id: BuildRequestId,
    build: &BuildConfig,
    imported: &[ImportedArtifact],
) -> Result<Vec<ReleaseArtifactInput>, BuildExecutionError> {
    imported
        .iter()
        .map(|artifact| {
            let declaration = build
                .artifacts
                .iter()
                .find(|declared| {
                    artifact.path.as_str() == declared.path
                        || (declared.kind == BuildArtifactKind::Directory
                            && artifact
                                .path
                                .as_str()
                                .strip_prefix(&declared.path)
                                .is_some_and(|suffix| suffix.starts_with('/')))
                })
                .ok_or(BuildExecutionError::UndeclaredOutput)?;
            Ok(ReleaseArtifactInput {
                id: stable_artifact_id(build_request_id, artifact.path.as_str()),
                path: artifact.path.clone(),
                kind: artifact.kind,
                mode: artifact.mode,
                content_hash: artifact.content_hash,
                size_bytes: artifact.size_bytes,
                media_type: declaration
                    .media_type
                    .clone()
                    .unwrap_or_else(|| String::from("application/octet-stream")),
                storage_key: artifact.storage_key,
            })
        })
        .collect()
}

pub(super) fn artifact_manifest(artifacts: &[ReleaseArtifactInput]) -> Value {
    Value::Array(
        artifacts
            .iter()
            .map(|artifact| {
                json!({
                    "id": artifact.id,
                    "path": artifact.path,
                    "kind": artifact_kind(artifact.kind),
                    "mode": artifact.mode,
                    "content_hash": artifact.content_hash,
                    "size_bytes": artifact.size_bytes,
                    "media_type": artifact.media_type,
                    "storage_key": artifact.storage_key,
                })
            })
            .collect(),
    )
}

pub(super) fn stable_artifact_id(
    build_request_id: BuildRequestId,
    path: &str,
) -> ReleaseArtifactId {
    let mut digest = Sha256::new();
    digest.update(b"hephaestus.release-artifact-id.v1");
    digest.update(build_request_id.as_uuid().as_bytes());
    digest.update((path.len() as u64).to_be_bytes());
    digest.update(path.as_bytes());
    let digest = digest.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x80;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    ReleaseArtifactId::from_uuid(Uuid::from_bytes(bytes))
}

const fn artifact_kind(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Executable => "executable",
        ArtifactKind::File => "file",
        ArtifactKind::Manifest => "manifest",
        ArtifactKind::BuildLog => "build_log",
    }
}

#[derive(Deserialize)]
struct StoredArtifact {
    id: ReleaseArtifactId,
    path: release_domain::ArtifactPath,
    kind: ArtifactKind,
    mode: u16,
    content_hash: release_domain::ContentHash,
    size_bytes: u64,
    media_type: String,
    storage_key: Uuid,
}

pub(super) fn stored_release_inputs(
    value: Value,
) -> Result<Vec<ReleaseArtifactInput>, BuildExecutionError> {
    let stored: Vec<StoredArtifact> =
        serde_json::from_value(value).map_err(|_| BuildExecutionError::StoredState)?;
    if stored.is_empty() {
        return Err(BuildExecutionError::StoredState);
    }
    Ok(stored
        .into_iter()
        .map(|artifact| ReleaseArtifactInput {
            id: artifact.id,
            path: artifact.path,
            kind: artifact.kind,
            mode: artifact.mode,
            content_hash: artifact.content_hash,
            size_bytes: artifact.size_bytes,
            media_type: artifact.media_type,
            storage_key: artifact.storage_key,
        })
        .collect())
}
