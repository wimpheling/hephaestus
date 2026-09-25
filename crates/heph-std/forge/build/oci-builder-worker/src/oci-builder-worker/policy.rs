use builder_catalog_domain::OciImageReference;
use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    ClaimedProductionJob, IsolatedOciBuild, OciImageProductionOutput, OciWorkerError,
    PreparedSource, RepositoryOciImageSourcePath,
};

pub fn isolated_request(
    job: &ClaimedProductionJob,
    source: &PreparedSource,
) -> Result<IsolatedOciBuild, OciWorkerError> {
    let checkout = canonical_directory(&source.checkout_root)?;
    let dockerfile = safe_child(&checkout, &job.dockerfile_path, false)?;
    let context = safe_child(&checkout, &job.context_path, true)?;
    let base_oci_layout = canonical_directory(&source.base_oci_layout)?;
    validate_tree(&context)?;
    let dockerfile_text = fs::read_to_string(&dockerfile).map_err(OciWorkerError::Filesystem)?;
    DockerfilePolicy::validate(&dockerfile_text)?;
    Ok(IsolatedOciBuild {
        job_id: job.id,
        image_id: job.image_id,
        project_id: job.project_id,
        dockerfile,
        checkout_root: checkout,
        context,
        base_oci_layout,
        base_reference: job.base_reference.clone(),
        network_disabled: true,
        ambient_credentials_disabled: true,
    })
}

pub fn canonical_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    if !path.is_absolute() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let canonical = fs::canonicalize(path).map_err(OciWorkerError::Filesystem)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    Ok(canonical)
}

pub fn safe_child(
    root: &Path,
    relative: &RepositoryOciImageSourcePath,
    directory: bool,
) -> Result<PathBuf, OciWorkerError> {
    let canonical =
        fs::canonicalize(root.join(relative.as_str())).map_err(OciWorkerError::Filesystem)?;
    if !canonical.starts_with(root) {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    let metadata = fs::symlink_metadata(&canonical).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || metadata.is_dir() != directory {
        return Err(OciWorkerError::UnsafeSourcePath);
    }
    Ok(canonical)
}

pub fn validate_tree(root: &Path) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(root).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let metadata = entry.file_type().map_err(OciWorkerError::Filesystem)?;
        if metadata.is_symlink() {
            return Err(OciWorkerError::UnsafeSourcePath);
        }
        if metadata.is_dir() {
            validate_tree(&entry.path())?;
        }
    }
    Ok(())
}

pub fn validate_output(
    job: &ClaimedProductionJob,
    output: &OciImageProductionOutput,
) -> Result<(), OciWorkerError> {
    if output
        .image_reference
        .digest()
        .map_err(|_| OciWorkerError::InvalidOutput)?
        != output.image_digest
        || output.attestation_reference.trim().is_empty()
        || output.attestation_reference.len() > 2048
        || output.scan_reference.trim().is_empty()
        || output.scan_reference.len() > 2048
        || output
            .sbom_reference
            .as_ref()
            .is_some_and(|reference| reference.trim().is_empty() || reference.len() > 2048)
        || job.base_reference.as_str().is_empty()
    {
        return Err(OciWorkerError::InvalidOutput);
    }
    Ok(())
}

pub fn digest_directory_name(reference: &OciImageReference) -> Result<String, OciWorkerError> {
    let digest = reference
        .digest()
        .map_err(|_| OciWorkerError::InvalidOutput)?;
    Ok(digest.as_str().replace(':', "-"))
}

pub fn bounded_reason(error: &OciWorkerError) -> String {
    let reason = error.to_string();
    reason.chars().take(2048).collect()
}

/// Dockerfile policy required before invoking any OCI implementation.
pub struct DockerfilePolicy;

impl DockerfilePolicy {
    /// Validates the restricted `heph-base` Dockerfile contract.
    ///
    /// # Errors
    ///
    /// Returns an error for an unapproved base, a remote context source, or a
    /// malformed `FROM` instruction.
    pub fn validate(source: &str) -> Result<(), OciWorkerError> {
        let mut stages = Vec::<String>::new();
        let mut saw_from = false;
        for raw_line in source.lines() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Multi-line instructions need a full Dockerfile parser. Rejecting
            // them is deliberately conservative: otherwise a remote source
            // could be hidden on a continuation line from this policy check.
            if line.ends_with('\\') {
                return Err(OciWorkerError::InvalidDockerfile);
            }
            let mut parts = line.split_whitespace();
            let Some(instruction) = parts.next() else {
                continue;
            };
            if instruction.eq_ignore_ascii_case("from") {
                let tokens: Vec<_> = parts
                    .filter(|part| !part.starts_with("--platform="))
                    .collect();
                let Some(image) = tokens.first() else {
                    return Err(OciWorkerError::InvalidDockerfile);
                };
                let allowed = if saw_from {
                    *image == "scratch" || stages.iter().any(|stage| stage == image)
                } else {
                    *image == "heph-base"
                };
                if !allowed {
                    return Err(OciWorkerError::UnapprovedDockerfileBase);
                }
                if tokens.len() >= 3 && tokens[1].eq_ignore_ascii_case("as") {
                    let name = tokens[2];
                    if !valid_stage_name(name) {
                        return Err(OciWorkerError::InvalidDockerfile);
                    }
                    stages.push(String::from(name));
                } else if tokens.len() != 1 {
                    return Err(OciWorkerError::InvalidDockerfile);
                }
                saw_from = true;
            } else if instruction.eq_ignore_ascii_case("add")
                || instruction.eq_ignore_ascii_case("copy")
            {
                let tokens: Vec<_> = parts.filter(|part| !part.starts_with("--")).collect();
                let lower = line.to_ascii_lowercase();
                if tokens.iter().any(|token| is_remote_source(token))
                    || lower.contains("http://")
                    || lower.contains("https://")
                    || lower.contains("git://")
                    || lower.contains("ssh://")
                {
                    return Err(OciWorkerError::RemoteDockerfileSource);
                }
            }
        }
        saw_from
            .then_some(())
            .ok_or(OciWorkerError::InvalidDockerfile)
    }
}

fn valid_stage_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_remote_source(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("git://")
        || lower.starts_with("ssh://")
}
