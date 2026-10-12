//! Explicit configured bootstrap and existing-only reopen.

use super::{
    DirectoryIdentity, FileIdentity, LOCK, Marker, OwnerGuard, PinnedCleanupRoots, ProviderOwner,
    SUPERVISOR, files,
};
use crate::config::LibkrunConfig;
use rustix::fs::{FlockOperation, Mode, OFlags};
use std::{fs::File, os::fd::AsRawFd, sync::Arc};
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
) -> Result<(Marker, File), VmError> {
    let (marker, file) = files::read_pinned_marker(runtime)?.ok_or_else(|| {
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
    Ok((marker, file))
}

fn finish(
    config: &LibkrunConfig,
    guard: &OwnerGuard,
    marker: Marker,
    marker_file: File,
    supervisor: File,
) -> Result<ProviderOwner, VmError> {
    let metadata_lock =
        files::readonly_original(&guard.runtime, LOCK, &guard.lock, config.service_uid)?;
    let owner = ProviderOwner {
        marker,
        marker_file: Arc::new(marker_file),
        metadata_lock: Arc::new(metadata_lock),
        supervisor,
        pinned_roots: Some(PinnedCleanupRoots::from_guard(guard)?),
    };
    owner.require_containment()?;
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

/// Read-only authorization tied to the original two directory descriptions.
pub struct ProspectiveOwner {
    runtime: File,
    cgroup: File,
    runtime_identity: DirectoryIdentity,
    cgroup_identity: DirectoryIdentity,
    namespace: uuid::Uuid,
    host_id: String,
    publication_started: bool,
    lock: Option<File>,
    supervisor: Option<File>,
}

fn require_pristine_runtime(runtime: &File) -> Result<(), VmError> {
    let mut entries = std::fs::read_dir(format!("/proc/self/fd/{}", runtime.as_raw_fd()))
        .map_err(files::io_error)?;
    if let Some(entry) = entries.next() {
        entry.map_err(files::io_error)?;
        return Err(files::invalid(
            "configured VM bootstrap requires an empty dedicated runtime root",
        ));
    }
    Ok(())
}

impl ProspectiveOwner {
    pub fn prepare(config: &LibkrunConfig, scope: &VmProviderOwnerScope) -> Result<Self, VmError> {
        let namespace = namespace(scope)?;
        let (runtime, cgroup) = roots(config)?;
        require_pristine_runtime(&runtime)?;
        files::require_unclassified_cgroups_absent(&cgroup)?;
        let prepared = Self {
            runtime_identity: DirectoryIdentity::read(&runtime)?,
            cgroup_identity: DirectoryIdentity::read(&cgroup)?,
            runtime,
            cgroup,
            namespace,
            host_id: scope.host_id().into(),
            publication_started: false,
            lock: None,
            supervisor: None,
        };
        prepared.validate_roots(config)?;
        Ok(prepared)
    }

    fn validate_roots(&self, config: &LibkrunConfig) -> Result<(), VmError> {
        if DirectoryIdentity::read(&self.runtime)? != self.runtime_identity
            || DirectoryIdentity::read(&self.cgroup)? != self.cgroup_identity
            || DirectoryIdentity::read(&files::open_root(
                &config.runtime_root,
                config.service_uid,
            )?)? != self.runtime_identity
            || DirectoryIdentity::read(&files::open_root(&config.cgroup_root, config.service_uid)?)?
                != self.cgroup_identity
        {
            return Err(files::invalid("prospective VM owner root identity changed"));
        }
        Ok(())
    }

    pub const fn publication_started(&self) -> bool {
        self.publication_started
    }

    pub fn publish(
        &mut self,
        config: &LibkrunConfig,
        after_reservation: impl FnOnce() -> Result<(), VmError>,
    ) -> Result<ProviderOwner, VmError> {
        self.validate_roots(config)?;
        require_pristine_runtime(&self.runtime)?;
        files::require_unclassified_cgroups_absent(&self.cgroup)?;
        // All writes are relative to the original descriptor, never a reopened
        // root. Identity checks cannot fence arbitrary same-UID path mutation.
        // Even a failed creating syscall is a publication attempt, not a
        // read-only rejection. Keep the original roots with that error too.
        self.publication_started = true;
        self.lock = Some(
            rustix::fs::openat(
                &self.runtime,
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
            .into(),
        );
        let lock = self.lock.as_ref().expect("publication installed its lock");
        files::check_file(lock, config.service_uid)?;
        rustix::fs::flock(lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(files::io_error)?;
        self.validate_roots(config)?;
        files::require_empty_runtime(&self.runtime)?;
        files::require_unclassified_cgroups_absent(&self.cgroup)?;
        // Retain the successful exclusive open before validation/sync/claim can
        // fail. The existing supervisor_file sequence otherwise drops on Err.
        self.supervisor = Some(
            rustix::fs::openat(
                &self.runtime,
                SUPERVISOR,
                OFlags::RDWR
                    | OFlags::CREATE
                    | OFlags::EXCL
                    | OFlags::NOFOLLOW
                    | OFlags::NONBLOCK
                    | OFlags::CLOEXEC,
                Mode::from_bits_truncate(0o600),
            )
            .map_err(files::io_error)?
            .into(),
        );
        let supervisor = self
            .supervisor
            .as_ref()
            .expect("publication installed its supervisor");
        files::check_file(supervisor, config.service_uid)?;
        supervisor.sync_all().map_err(files::io_error)?;
        self.runtime.sync_all().map_err(files::io_error)?;
        files::claim_supervisor(supervisor)?;
        after_reservation()?;
        let marker = Marker {
            version: 3,
            namespace: self.namespace,
            host_id: self.host_id.clone(),
            runtime: self.runtime_identity.clone(),
            cgroup: self.cgroup_identity.clone(),
            supervisor: FileIdentity::read(supervisor)?,
        };
        self.validate_roots(config)?;
        let marker_file = files::write_marker(&self.runtime, &marker)?;
        let guard = OwnerGuard {
            runtime: self.runtime.try_clone().map_err(files::io_error)?,
            cgroup: self.cgroup.try_clone().map_err(files::io_error)?,
            lock: lock.try_clone().map_err(files::io_error)?,
        };
        finish(
            config,
            &guard,
            marker,
            marker_file,
            supervisor.try_clone().map_err(files::io_error)?,
        )
    }
}

impl ProviderOwner {
    #[cfg(test)]
    pub fn bootstrap(
        config: &LibkrunConfig,
        scope: &VmProviderOwnerScope,
    ) -> Result<Self, VmError> {
        ProspectiveOwner::prepare(config, scope)?.publish(config, || Ok(()))
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
            lock,
        };
        let (marker, marker_file) = exact_marker(&guard.runtime, &guard.cgroup, scope)?;
        if marker != before.0 || FileIdentity::read(&marker_file)? != FileIdentity::read(&before.1)?
        {
            return Err(files::invalid(
                "configured VM metadata changed during reopen",
            ));
        }
        let supervisor = readonly_supervisor(&guard.runtime, config.service_uid)?;
        if FileIdentity::read(&supervisor)? != marker.supervisor {
            return Err(files::invalid("configured VM supervisor identity differs"));
        }
        files::claim_supervisor(&supervisor)?;
        finish(config, &guard, marker, marker_file, supervisor)
    }
}
