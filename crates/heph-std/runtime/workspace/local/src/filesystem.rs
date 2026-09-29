use crate::common::{
    Digest, File, LocalWorkspaceError, OpenOptions, Path, PermissionsExt, Sha256, Uuid, Write, fs,
    io_error, symlink,
};

pub fn write_owner_marker(root: &Path, owner: &str) -> Result<(), LocalWorkspaceError> {
    Uuid::parse_str(owner).map_err(|_| {
        LocalWorkspaceError::UnsafePath(String::from("workspace owner is not an opaque ID"))
    })?;
    write_new_file(&root.join(".hephaestus-workspace"), owner.as_bytes(), false)
}

pub fn copy_tree(source: &Path, destination: &Path) -> Result<(), LocalWorkspaceError> {
    fs::create_dir(destination).map_err(io_error)?;
    for entry in fs::read_dir(source).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            copy_tree(&source_path, &destination_path)?;
        } else if metadata.file_type().is_file() {
            fs::copy(&source_path, &destination_path).map_err(io_error)?;
            fs::set_permissions(
                &destination_path,
                fs::Permissions::from_mode(if metadata.permissions().mode() & 0o111 != 0 {
                    0o755
                } else {
                    0o644
                }),
            )
            .map_err(io_error)?;
        } else if metadata.file_type().is_symlink() {
            symlink(
                fs::read_link(&source_path).map_err(io_error)?,
                &destination_path,
            )
            .map_err(io_error)?;
        } else {
            return Err(LocalWorkspaceError::InvalidSource(String::from(
                "source materialization contains an unsupported object",
            )));
        }
    }
    Ok(())
}

pub fn make_source_read_only(path: &Path) -> Result<(), LocalWorkspaceError> {
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            make_source_read_only(&child)?;
        } else if metadata.file_type().is_file() {
            let executable = metadata.permissions().mode() & 0o111 != 0;
            fs::set_permissions(
                &child,
                fs::Permissions::from_mode(if executable { 0o555 } else { 0o444 }),
            )
            .map_err(io_error)?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o555)).map_err(io_error)
}

pub fn fsync_tree(path: &Path) -> Result<(), LocalWorkspaceError> {
    for entry in fs::read_dir(path).map_err(io_error)? {
        let entry = entry.map_err(io_error)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(io_error)?;
        if metadata.file_type().is_dir() {
            fsync_tree(&child)?;
        } else if metadata.file_type().is_file() {
            File::open(&child)
                .map_err(io_error)?
                .sync_all()
                .map_err(io_error)?;
        }
    }
    sync_directory(path)
}

pub fn sync_directory(path: &Path) -> Result<(), LocalWorkspaceError> {
    File::open(path)
        .map_err(io_error)?
        .sync_all()
        .map_err(io_error)
}

pub fn write_new_file(
    path: &Path,
    bytes: &[u8],
    executable: bool,
) -> Result<(), LocalWorkspaceError> {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .map_err(io_error)?;
    file.write_all(bytes).map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
    )
    .map_err(io_error)
}

pub fn utf8_path(path: &Path) -> Result<String, LocalWorkspaceError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| LocalWorkspaceError::UnsafePath(String::from("path is not UTF-8")))
}

pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut value = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut value, "{byte:02x}").expect("writing to a String cannot fail");
    }
    value
}
