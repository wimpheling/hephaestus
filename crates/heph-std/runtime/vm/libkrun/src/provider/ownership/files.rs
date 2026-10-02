use super::{LOCK, MARKER, Marker, OwnerGuard};
use crate::config::LibkrunConfig;
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::{
    fs::{self, File},
    io::{Read, Write},
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::Path,
};
use vm_trait::VmError;

pub(super) fn invalid(reason: &str) -> VmError {
    VmError::InvalidSpec {
        field: "provider_owner".into(),
        reason: reason.into(),
    }
}

pub(super) fn io_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: "libkrun".into(),
        code: "owner_metadata".into(),
        source: Box::new(error),
    }
}

pub(super) fn open_root(path: &Path, uid: u32) -> Result<File, VmError> {
    if !path.is_absolute() || fs::canonicalize(path).map_err(io_error)? != path {
        return Err(invalid(
            "VM ownership roots must be canonical directories without symlink components",
        ));
    }
    let file: File = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?
    .into();
    let metadata = file.metadata().map_err(io_error)?;
    if metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(invalid(
            "VM ownership roots require the configured UID and no group/world write access",
        ));
    }
    Ok(file)
}

fn check_file(file: &File, uid: u32) -> Result<(), VmError> {
    let metadata = file.metadata().map_err(io_error)?;
    if !metadata.is_file()
        || metadata.uid() != uid
        || metadata.nlink() != 1
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(invalid(
            "VM owner metadata must be a private single-link regular file",
        ));
    }
    Ok(())
}

pub(super) fn locked_root(
    config: &LibkrunConfig,
    initializing: bool,
) -> Result<OwnerGuard, VmError> {
    let runtime = open_root(&config.runtime_root, config.service_uid)?;
    let cgroup = open_root(&config.cgroup_root, config.service_uid)?;
    // Validation must never create metadata in a redirected/foreign root.
    let mut flags = OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
    if initializing {
        flags |= OFlags::CREATE;
    }
    let lock: File = rustix::fs::openat(&runtime, LOCK, flags, Mode::from_bits_truncate(0o600))
        .map_err(io_error)?
        .into();
    check_file(&lock, config.service_uid)?;
    if initializing {
        runtime.sync_all().map_err(io_error)?;
    }
    let operation = if initializing {
        FlockOperation::LockExclusive
    } else {
        FlockOperation::NonBlockingLockShared
    };
    rustix::fs::flock(&lock, operation).map_err(io_error)?;
    Ok(OwnerGuard {
        runtime,
        cgroup,
        _lock: lock,
    })
}

pub(super) fn read_marker(root: &File) -> Result<Option<Marker>, VmError> {
    let file = match rustix::fs::openat(
        root,
        MARKER,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(file) => File::from(file),
        Err(rustix::io::Errno::NOENT) => return Ok(None),
        Err(error) => return Err(io_error(error)),
    };
    check_file(&file, root.metadata().map_err(io_error)?.uid())?;
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes).map_err(io_error)?;
    if bytes.len() > 4096 {
        return Err(invalid("VM owner metadata exceeds its bounded format"));
    }
    serde_json::from_slice(&bytes).map(Some).map_err(io_error)
}

pub(super) fn write_marker(root: &File, marker: &Marker) -> Result<(), VmError> {
    let mut file: File = rustix::fs::openat(
        root,
        MARKER,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_bits_truncate(0o600),
    )
    .map_err(io_error)?
    .into();
    file.write_all(&serde_json::to_vec(marker).map_err(io_error)?)
        .map_err(io_error)?;
    file.sync_all().map_err(io_error)?;
    root.sync_all().map_err(io_error)
}

pub(super) fn require_empty_runtime(root: &File) -> Result<(), VmError> {
    for entry in fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd())).map_err(io_error)? {
        if entry.map_err(io_error)?.file_name() != LOCK {
            return Err(invalid(
                "VM runtime root has unclassified resources; automatic ownership adoption is unavailable",
            ));
        }
    }
    Ok(())
}

pub(super) fn require_unclassified_cgroups_absent(root: &File) -> Result<(), VmError> {
    for entry in fs::read_dir(format!("/proc/self/fd/{}", root.as_raw_fd())).map_err(io_error)? {
        let kind = entry.map_err(io_error)?.file_type().map_err(io_error)?;
        // Kernel control files belong in a delegated root. Old VM directories
        // and redirected entries cannot be adopted implicitly.
        if !kind.is_file() {
            return Err(invalid(
                "delegated cgroup root has unclassified resources; automatic ownership adoption is unavailable",
            ));
        }
    }
    Ok(())
}
