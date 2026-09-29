//! Secure result-artifact preview loading.

use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::Read, os::unix::fs::OpenOptionsExt, path::Path};
use uuid::Uuid;

use super::model::{MAX_RESULT_PREVIEW_BYTES, RunArtifact, RunError};

#[derive(Clone)]
pub(super) struct PreviewArtifact {
    pub(super) kind: String,
    pub(super) size_bytes: i64,
    pub(super) sha256: String,
    pub(super) storage_key: String,
}

impl From<&RunArtifact> for PreviewArtifact {
    fn from(value: &RunArtifact) -> Self {
        Self {
            kind: value.kind.clone(),
            size_bytes: value.size_bytes,
            sha256: value.sha256.clone(),
            storage_key: value.storage_key.clone(),
        }
    }
}

#[derive(Default)]
pub(super) struct ResultPreviews {
    pub(super) patch: Option<String>,
    pub(super) manifest: Option<String>,
}

pub(super) fn load_previews(
    root: &Path,
    run_id: Uuid,
    artifacts: &[PreviewArtifact],
) -> Result<ResultPreviews, RunError> {
    let patch = artifacts
        .iter()
        .find(|artifact| artifact.kind == "patch")
        .map(|artifact| read_preview(root, run_id, artifact, "patch"))
        .transpose()?
        .flatten();
    let manifest = artifacts
        .iter()
        .find(|artifact| artifact.kind == "manifest")
        .map(|artifact| read_preview(root, run_id, artifact, "json"))
        .transpose()?
        .flatten();
    Ok(ResultPreviews { patch, manifest })
}

pub(super) fn read_preview(
    root: &Path,
    run_id: Uuid,
    artifact: &PreviewArtifact,
    extension: &str,
) -> Result<Option<String>, RunError> {
    let recorded_size =
        u64::try_from(artifact.size_bytes).map_err(|_| RunError::PreviewUnavailable)?;
    if recorded_size > MAX_RESULT_PREVIEW_BYTES {
        return Ok(None);
    }
    let expected_key = format!(
        "{run_id}/{}-{}.{}",
        artifact.kind, artifact.sha256, extension
    );
    if artifact.storage_key != expected_key
        || artifact.sha256.len() != 64
        || !artifact.sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RunError::PreviewUnavailable);
    }
    let directory = root.join(run_id.to_string());
    let directory_metadata =
        std::fs::symlink_metadata(&directory).map_err(|_| RunError::PreviewUnavailable)?;
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(RunError::PreviewUnavailable);
    }
    let path = root.join(&artifact.storage_key);
    let metadata = std::fs::symlink_metadata(&path).map_err(|_| RunError::PreviewUnavailable)?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() != recorded_size
    {
        return Err(RunError::PreviewUnavailable);
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(0o400_000 | 0o2_000_000)
        .open(path)
        .map_err(|_| RunError::PreviewUnavailable)?;
    let capacity = usize::try_from(recorded_size).map_err(|_| RunError::PreviewUnavailable)?;
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_RESULT_PREVIEW_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RunError::PreviewUnavailable)?;
    let maximum_size =
        usize::try_from(MAX_RESULT_PREVIEW_BYTES).map_err(|_| RunError::PreviewUnavailable)?;
    if bytes.len() != capacity || bytes.len() > maximum_size {
        return Err(RunError::PreviewUnavailable);
    }
    let actual_hash = format!("{:x}", Sha256::digest(&bytes));
    if actual_hash != artifact.sha256 {
        return Err(RunError::PreviewUnavailable);
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| RunError::PreviewUnavailable)
}
