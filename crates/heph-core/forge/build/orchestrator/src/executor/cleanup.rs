use super::{BuildExecutionError, Path};
use std::{fs, os::unix::fs::PermissionsExt};

pub(super) fn cleanup_workspace(root: &Path) -> Result<(), BuildExecutionError> {
    if !root.exists() {
        return Ok(());
    }
    make_removable(root)?;
    fs::remove_dir_all(root).map_err(filesystem)
}

fn make_removable(root: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(root).map_err(filesystem)?;
    if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
        return Err(BuildExecutionError::UnsafeWorkspace);
    }
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).map_err(filesystem)?;
    for entry in fs::read_dir(root).map_err(filesystem)? {
        let entry = entry.map_err(filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(filesystem)?;
        if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
            make_removable(&entry.path())?;
        } else if !metadata.file_type().is_file() {
            return Err(BuildExecutionError::UnsafeWorkspace);
        }
    }
    Ok(())
}

pub(super) fn validate_private_directory(path: &Path) -> Result<(), BuildExecutionError> {
    let metadata = fs::symlink_metadata(path).map_err(filesystem)?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(BuildExecutionError::InvalidConfiguration);
    }
    Ok(())
}

pub(super) fn filesystem(_error: std::io::Error) -> BuildExecutionError {
    BuildExecutionError::Filesystem
}
