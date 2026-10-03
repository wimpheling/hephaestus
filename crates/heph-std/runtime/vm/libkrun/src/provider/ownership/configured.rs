//! Explicit configured bootstrap and existing-only reopen.

use super::{
    DirectoryIdentity, FileIdentity, LOCK, Marker, OwnerGuard, PinnedCleanupRoots, ProviderOwner,
    SUPERVISOR, files,
};
use crate::config::LibkrunConfig;
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::{fs::File, os::fd::AsRawFd};
use vm_trait::{VmError, VmProviderOwnerScope};

fn namespace(scope: &VmProviderOwnerScope) -> Result<uuid::Uuid, VmError> {
    let id = uuid::Uuid::parse_str(scope.namespace())
        .map_err(|_| files::invalid("configured VM namespace must be a canonical nonnil UUID"))?;
    if id.is_nil() || id.to_string() != scope.namespace() {
        return Err(files::invalid(
            "configured VM namespace must be a canonical nonnil UUID",
        ));
    }
    Ok(id)
}

fn roots(config: &LibkrunConfig) -> Result<(File, File), VmError> {
    Ok((
        files::open_root(&config.runtime_root, config.service_uid)?,
        files::open_root(&config.cgroup_root, config.service_uid)?,
    ))
}

fn exact_marker(
    runtime: &File,
    cgroup: &File,
    scope: &VmProviderOwnerScope,
) -> Result<Marker, VmError> {
    let marker = files::read_marker(runtime)?.ok_or_else(|| {
        files::invalid("configured VM owner marker is missing; reopen never bootstraps")
    })?;
    if marker.namespace != namespace(scope)?
        || marker.host_id != scope.host_id()
        || marker.runtime != DirectoryIdentity::read(runtime)?
        || marker.cgroup != DirectoryIdentity::read(cgroup)?
    {
        return Err(files::invalid(
            "configured VM owner scope or root identity differs",
        ));
    }
    Ok(marker)
}

fn finish(
    config: &LibkrunConfig,
    guard: &OwnerGuard,
    marker: Marker,
    supervisor: File,
) -> Result<ProviderOwner, VmError> {
    let owner = ProviderOwner {
        marker,
        supervisor,
        pinned_roots: Some(PinnedCleanupRoots::from_guard(guard)?),
    };
    owner.validate_guard(config, guard)?;
    Ok(owner)
}

pub(super) fn readonly_supervisor(root: &File, uid: u32) -> Result<File, VmError> {
    let file: File = rustix::fs::openat(
        root,
        SUPERVISOR,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(files::io_error)?
    .into();
    files::check_file(&file, uid)?;
    Ok(file)
}

impl ProviderOwner {
    pub fn bootstrap(
        config: &LibkrunConfig,
        scope: &VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        let namespace = namespace(scope)?;
        let (runtime, cgroup) = roots(config)?;
        // Explicit bootstrap accepts no prior metadata, including an incomplete
        // earlier attempt. Only kernel control files may exist in the cgroup.
        if std::fs::read_dir(format!("/proc/self/fd/{}", runtime.as_raw_fd()))
            .map_err(files::io_error)?
            .next()
            .is_some()
        {
            return Err(files::invalid(
                "configured VM bootstrap requires an empty dedicated runtime root",
            ));
        }
        files::require_unclassified_cgroups_absent(&cgroup)?;
        let lock: File = rustix::fs::openat(
            &runtime,
            LOCK,
            OFlags::RDWR
                | OFlags::CREATE
                | OFlags::EXCL
                | OFlags::NOFOLLOW
                | OFlags::NONBLOCK
                | OFlags::CLOEXEC,
            Mode::from_bits_truncate(0o600),
        )
        .map_err(files::io_error)?
        .into();
        files::check_file(&lock, config.service_uid)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(files::io_error)?;
        let guard = OwnerGuard {
            runtime,
            cgroup,
            _lock: lock,
        };
        files::require_empty_runtime(&guard.runtime)?;
        files::require_unclassified_cgroups_absent(&guard.cgroup)?;
        let supervisor = files::supervisor_file(&guard.runtime, true, config.service_uid)?;
        files::claim_supervisor(&supervisor)?;
        let marker = Marker {
            version: 2,
            namespace,
            host_id: scope.host_id().into(),
            runtime: DirectoryIdentity::read(&guard.runtime)?,
            cgroup: DirectoryIdentity::read(&guard.cgroup)?,
            supervisor: FileIdentity::read(&supervisor)?,
        };
        files::write_marker(&guard.runtime, &marker)?;
        finish(config, &guard, marker, supervisor)
    }

    pub fn open_existing(
        config: &LibkrunConfig,
        scope: &VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        namespace(scope)?;
        let (runtime, cgroup) = roots(config)?;
        // Check the configured identities before opening any foreign lock.
        let before = exact_marker(&runtime, &cgroup, scope)?;
        let lock: File = rustix::fs::openat(
            &runtime,
            LOCK,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(files::io_error)?
        .into();
        files::check_file(&lock, config.service_uid)?;
        rustix::fs::flock(&lock, FlockOperation::NonBlockingLockShared).map_err(files::io_error)?;
        let guard = OwnerGuard {
            runtime,
            cgroup,
            _lock: lock,
        };
        let marker = exact_marker(&guard.runtime, &guard.cgroup, scope)?;
        if marker != before {
            return Err(files::invalid(
                "configured VM metadata changed during reopen",
            ));
        }
        let supervisor = readonly_supervisor(&guard.runtime, config.service_uid)?;
        if FileIdentity::read(&supervisor)? != marker.supervisor {
            return Err(files::invalid("configured VM supervisor identity differs"));
        }
        files::claim_supervisor(&supervisor)?;
        finish(config, &guard, marker, supervisor)
    }
}
