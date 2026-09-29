//! Private filesystem and OCI layout safeguards.

use super::{
    constants::{
        BUILDER_SCRATCH_BYTES, BUILDER_SOURCE_GUEST_PATH, ScratchDisk, VerifiedVmOciOutput,
    },
    output::canonical_directory,
};
use builder_catalog_domain::OciDigest;
use oci_builder_worker::OciWorkerError;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, Stdio},
};

pub fn prepare_scratch_disk(
    root: &Path,
    mkfs_ext4: &Path,
    job_id: uuid::Uuid,
) -> Result<ScratchDisk, OciWorkerError> {
    let path = root.join(format!("{job_id}.raw"));
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(OciWorkerError::Filesystem)?;
    if let Err(error) = file.set_len(BUILDER_SCRATCH_BYTES) {
        let _ignored = fs::remove_file(&path);
        return Err(OciWorkerError::Filesystem(error));
    }
    let filesystem_uuid = uuid::Uuid::new_v4();
    let filesystem_uuid_text = filesystem_uuid.to_string();
    let status = ProcessCommand::new(mkfs_ext4)
        // Ubuntu ships mkfs.ext4 as a symlink to mke2fs. The caller stores
        // only the canonical executable, so select ext4 explicitly instead of
        // relying on argv[0] to choose the filesystem type.
        .args(["-t", "ext4", "-q", "-F", "-U", &filesystem_uuid_text])
        .arg(&path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(OciWorkerError::Filesystem)?;
    if !status.success() {
        let _ignored = fs::remove_file(&path);
        return Err(OciWorkerError::InvalidConfiguration);
    }
    Ok(ScratchDisk {
        path,
        filesystem_uuid,
    })
}

pub fn initialize_private_directory(path: &Path) -> Result<PathBuf, OciWorkerError> {
    fs::create_dir_all(path).map_err(OciWorkerError::Filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(OciWorkerError::Filesystem)?;
    let path = canonical_directory(path)?;
    let mode = fs::metadata(&path)
        .map_err(OciWorkerError::Filesystem)?
        .permissions()
        .mode();
    (mode.trailing_zeros() >= 6)
        .then_some(path)
        .ok_or(OciWorkerError::InvalidConfiguration)
}

pub fn prepare_empty_directory(path: &Path) -> Result<(), OciWorkerError> {
    if path.exists() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    fs::create_dir(path).map_err(OciWorkerError::Filesystem)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(OciWorkerError::Filesystem)
}

pub fn remove_private_directory(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    restore_directory_tree_owner_write(path)?;
    fs::remove_dir_all(path).map_err(OciWorkerError::Filesystem)
}

pub fn restore_directory_tree_owner_write(path: &Path) -> Result<(), OciWorkerError> {
    let metadata = fs::symlink_metadata(path).map_err(OciWorkerError::Filesystem)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        if entry
            .file_type()
            .map_err(OciWorkerError::Filesystem)?
            .is_dir()
        {
            restore_directory_tree_owner_write(&entry.path())?;
        }
    }
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(metadata.permissions().mode() | 0o700),
    )
    .map_err(OciWorkerError::Filesystem)
}

pub fn guest_child_path(
    root: &Path,
    child: &Path,
    allow_root: bool,
) -> Result<PathBuf, OciWorkerError> {
    let relative = child
        .strip_prefix(root)
        .map_err(|_| OciWorkerError::UnsafeSourcePath)?;
    if relative.as_os_str().is_empty() {
        return allow_root
            .then(|| PathBuf::from(BUILDER_SOURCE_GUEST_PATH))
            .ok_or(OciWorkerError::UnsafeSourcePath);
    }
    Ok(PathBuf::from(BUILDER_SOURCE_GUEST_PATH).join(relative))
}

pub fn seal_candidate_layout(path: &Path) -> Result<(), OciWorkerError> {
    let index = path.join("index.json");
    let layout = path.join("oci-layout");
    if !index.is_file() || !layout.is_file() || symlinked_tree(path)? {
        return Err(OciWorkerError::InvalidOutput);
    }
    readonly_tree(path)
}

pub fn symlinked_tree(path: &Path) -> Result<bool, OciWorkerError> {
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(OciWorkerError::Filesystem)?;
        if metadata.file_type().is_symlink()
            || (metadata.is_dir() && symlinked_tree(&entry.path())?)
        {
            return Ok(true);
        }
    }
    Ok(false)
}

pub fn readonly_tree(path: &Path) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(path).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(OciWorkerError::Filesystem)?;
        if metadata.is_dir() {
            readonly_tree(&child)?;
            fs::set_permissions(&child, fs::Permissions::from_mode(0o500))
                .map_err(OciWorkerError::Filesystem)?;
        } else {
            fs::set_permissions(&child, fs::Permissions::from_mode(0o400))
                .map_err(OciWorkerError::Filesystem)?;
        }
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o500)).map_err(OciWorkerError::Filesystem)
}

pub fn verified_vm_output(
    layout: &Path,
    verification: &Path,
) -> Result<VerifiedVmOciOutput, OciWorkerError> {
    let evidence = verification.join("evidence");
    let sbom = regular_output_file(&evidence, "sbom.spdx.json")?;
    let scan = regular_output_file(&evidence, "vulnerability-scan.json")?;
    let rootfs = verification.join("rootfs");
    let rootfs_metadata = fs::symlink_metadata(&rootfs).map_err(OciWorkerError::Filesystem)?;
    if rootfs_metadata.file_type().is_symlink() || !rootfs_metadata.is_dir() {
        return Err(OciWorkerError::InvalidOutput);
    }
    let digest_text = fs::read_to_string(regular_output_file(verification, "manifest-digest")?)
        .map_err(OciWorkerError::Filesystem)?;
    let manifest_digest = OciDigest::parse(digest_text.trim().to_owned())
        .map_err(|_| OciWorkerError::InvalidOutput)?;
    Ok(VerifiedVmOciOutput {
        layout: layout.to_path_buf(),
        sbom,
        scan,
        rootfs,
        manifest_digest,
    })
}

pub fn regular_output_file(parent: &Path, name: &str) -> Result<PathBuf, OciWorkerError> {
    let path = parent.join(name);
    let metadata = fs::symlink_metadata(&path).map_err(OciWorkerError::Filesystem)?;
    (!metadata.file_type().is_symlink() && metadata.is_file())
        .then_some(path)
        .ok_or(OciWorkerError::InvalidOutput)
}

pub fn copy_verified_rootfs(source: &Path, destination: &Path) -> Result<(), OciWorkerError> {
    let destination_metadata =
        fs::symlink_metadata(destination).map_err(OciWorkerError::Filesystem)?;
    if destination_metadata.file_type().is_symlink() || !destination_metadata.is_dir() {
        return Err(OciWorkerError::UnsafeMaterializationPath);
    }
    copy_tree_without_following_links(source, destination)
}

pub fn copy_tree_without_following_links(
    source: &Path,
    destination: &Path,
) -> Result<(), OciWorkerError> {
    for entry in fs::read_dir(source).map_err(OciWorkerError::Filesystem)? {
        let entry = entry.map_err(OciWorkerError::Filesystem)?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = fs::symlink_metadata(&source_path).map_err(OciWorkerError::Filesystem)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            fs::create_dir(&destination_path).map_err(OciWorkerError::Filesystem)?;
            fs::set_permissions(&destination_path, metadata.permissions())
                .map_err(OciWorkerError::Filesystem)?;
            copy_tree_without_following_links(&source_path, &destination_path)?;
        } else if file_type.is_file() {
            fs::copy(&source_path, &destination_path).map_err(OciWorkerError::Filesystem)?;
            fs::set_permissions(&destination_path, metadata.permissions())
                .map_err(OciWorkerError::Filesystem)?;
        } else if file_type.is_symlink() {
            let target = fs::read_link(&source_path).map_err(OciWorkerError::Filesystem)?;
            std::os::unix::fs::symlink(target, &destination_path)
                .map_err(OciWorkerError::Filesystem)?;
        } else {
            return Err(OciWorkerError::InvalidOutput);
        }
    }
    Ok(())
}
