use super::PROVIDER_NAME;
use std::{
    fs,
    path::{Path, PathBuf},
};
use vm_trait::{VmError, VmId};

pub fn validate_id(id: &VmId) -> Result<(), VmError> {
    if id.0.is_empty()
        || id.0.len() > 64
        || !id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return invalid(
            "id",
            "must be 1-64 ASCII letters, digits, hyphens, or underscores",
        );
    }
    Ok(())
}

pub(super) fn canonical_allowed(
    field: &str,
    path: &Path,
    roots: &[PathBuf],
    kind: PathKind,
) -> Result<PathBuf, VmError> {
    validate_absolute(field, path)?;
    let canonical =
        fs::canonicalize(path).map_err(|error| invalid_error(field, error.to_string()))?;
    let allowed = roots.iter().any(|root| {
        fs::canonicalize(root).is_ok_and(|allowed_root| canonical.starts_with(allowed_root))
    });
    if !allowed {
        return invalid(field, "escapes configured allowed roots");
    }
    let metadata = fs::metadata(&canonical).map_err(provider_io)?;
    let valid_kind = match kind {
        PathKind::Directory => metadata.is_dir(),
        PathKind::File => metadata.is_file(),
    };
    if !valid_kind {
        return invalid(field, kind.description());
    }
    Ok(canonical)
}

pub(super) fn validate_absolute(field: &str, path: &Path) -> Result<(), VmError> {
    if !path.is_absolute() {
        return invalid(field, "must be an absolute path");
    }
    Ok(())
}

pub(super) fn validate_no_nul(field: &str, value: &str) -> Result<(), VmError> {
    if value.as_bytes().contains(&0) {
        return invalid(field, "must not contain NUL");
    }
    Ok(())
}

pub(super) fn invalid<T>(
    field: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, VmError> {
    Err(invalid_error(field, reason))
}

pub(super) fn invalid_error(field: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::InvalidSpec {
        field: field.into(),
        reason: reason.into(),
    }
}

pub(super) fn unsupported<T>(feature: impl Into<String>) -> Result<T, VmError> {
    Err(VmError::Unsupported {
        feature: feature.into(),
        provider: PROVIDER_NAME.to_owned(),
    })
}

pub(super) fn unavailable<T>(
    resource: impl Into<String>,
    reason: impl Into<String>,
) -> Result<T, VmError> {
    Err(unavailable_error(resource, reason))
}

pub(super) fn unavailable_error(resource: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::Unavailable {
        resource: resource.into(),
        reason: reason.into(),
    }
}

pub(super) fn provider_io(source: std::io::Error) -> VmError {
    VmError::Provider {
        provider: PROVIDER_NAME.to_owned(),
        code: "host-io".to_owned(),
        source: Box::new(source),
    }
}

#[derive(Clone, Copy)]
pub(super) enum PathKind {
    Directory,
    File,
}

impl PathKind {
    const fn description(self) -> &'static str {
        match self {
            Self::Directory => "must be a directory",
            Self::File => "must be a regular file",
        }
    }
}
