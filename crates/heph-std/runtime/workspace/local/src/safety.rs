use crate::common::{
    Component, LocalWorkspaceConfig, LocalWorkspaceError, OsStr, Path, PermissionsExt, Uuid, fs,
    fsync_tree, io_error, sync_directory,
};

pub fn validate_relative_path(path: &str) -> Result<(), LocalWorkspaceError> {
    let path = Path::new(path);
    if path.is_absolute() || path.components().next().is_none() {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "invalid repository path {}",
            path.display()
        )));
    }
    for component in path.components() {
        match component {
            Component::Normal(value) if !value.eq_ignore_ascii_case(OsStr::new(".git")) => {}
            _ => {
                return Err(LocalWorkspaceError::UnsafePath(format!(
                    "unsafe repository path {}",
                    path.display()
                )));
            }
        }
    }
    Ok(())
}

pub fn validate_name(name: &str) -> Result<(), LocalWorkspaceError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "unsafe workspace entry {name:?}"
        )));
    }
    Ok(())
}

pub fn validate_symlink_target(target: &str) -> Result<(), LocalWorkspaceError> {
    let path = Path::new(target);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "unsafe symlink target {target:?}"
        )));
    }
    Ok(())
}

pub fn enforce_bytes(
    config: &LocalWorkspaceConfig,
    total: &mut u64,
    size: u64,
) -> Result<(), LocalWorkspaceError> {
    if size > config.limits.max_file_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "file exceeds configured byte limit",
        )));
    }
    *total = total
        .checked_add(size)
        .ok_or_else(|| LocalWorkspaceError::Quota(String::from("workspace size overflow")))?;
    if *total > config.limits.max_total_bytes {
        return Err(LocalWorkspaceError::Quota(String::from(
            "workspace exceeds configured aggregate byte limit",
        )));
    }
    Ok(())
}

pub fn ensure_workspace_path(
    config: &LocalWorkspaceConfig,
    path: &Path,
    class: &str,
) -> Result<(), LocalWorkspaceError> {
    let expected_parent = config.workspace_root.join(class);
    if path.parent() != Some(expected_parent.as_path()) || path.file_name().is_none() {
        return Err(LocalWorkspaceError::UnsafePath(format!(
            "workspace path {} is outside {class}",
            path.display()
        )));
    }
    Ok(())
}

pub fn seal_workspace(active: &Path, sealed: &Path) -> Result<(), LocalWorkspaceError> {
    if sealed.exists() {
        if active.exists() {
            return Err(LocalWorkspaceError::Integrity(String::from(
                "active and sealed workspace both exist",
            )));
        }
        return Ok(());
    }
    fsync_tree(active)?;
    let active_parent = active.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("active workspace root missing"))
    })?;
    let sealed_parent = sealed
        .parent()
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("sealed root missing")))?;
    fs::rename(active, sealed).map_err(io_error)?;
    sync_directory(active_parent)?;
    sync_directory(sealed_parent)
}

pub fn remove_owned_workspace(
    config: &LocalWorkspaceConfig,
    path: &Path,
    class: &str,
) -> Result<(), LocalWorkspaceError> {
    ensure_workspace_path(config, path, class)?;
    if !fs::symlink_metadata(path)
        .map_err(io_error)?
        .file_type()
        .is_dir()
    {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "workspace cleanup target is not a directory",
        )));
    }
    let marker = fs::read_to_string(path.join(".hephaestus-workspace")).map_err(io_error)?;
    let owner = marker.trim();
    Uuid::parse_str(owner).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("workspace ownership marker is not an ID"))
    })?;
    let file_name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
    if file_name != owner && !file_name.starts_with(&format!("{owner}.")) {
        return Err(LocalWorkspaceError::UnsafePath(String::from(
            "workspace ownership marker is invalid",
        )));
    }
    make_tree_removable(path)?;
    let parent = path.parent().ok_or_else(|| {
        LocalWorkspaceError::UnsafePath(String::from("workspace cleanup parent is missing"))
    })?;
    fs::remove_dir_all(path).map_err(io_error)?;
    sync_directory(parent)
}

pub fn make_tree_removable(path: &Path) -> Result<(), LocalWorkspaceError> {
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(io_error)?;
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        if fs::symlink_metadata(&child)
            .map_err(io_error)?
            .file_type()
            .is_dir()
        {
            make_tree_removable(&child)?;
        }
    }
    Ok(())
}
