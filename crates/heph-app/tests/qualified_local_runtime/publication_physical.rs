//! Passive worker observations tied to the actual publication provider roots.
//!
//! These reads neither clean resources nor certify joined daemon shutdown. A
//! pre-start worker and its exact cgroup are observed, not every native child PID.

use std::{
    fs::{self, File},
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    path::{Path, PathBuf},
    sync::Arc,
};

use rustix::fs::{AtFlags, Mode, OFlags};
use vm_trait::{VmError, VmId, VmProviderOwnerScope};

use super::{ObservedWorkerExit, ProvisionAttempt, ProvisionOutcome};

struct PinnedRoot {
    path: PathBuf,
    file: File,
}

impl PinnedRoot {
    fn new(path: &Path) -> Result<Self, VmError> {
        if !path.is_absolute() || fs::canonicalize(path).map_err(io_error)? != path {
            return Err(held("publication observer root is not canonical"));
        }
        let file = open_directory(path)?;
        let metadata = file.metadata().map_err(io_error)?;
        if metadata.uid() != rustix::process::geteuid().as_raw() || metadata.mode() & 0o022 != 0 {
            return Err(held("publication observer root ownership is unsafe"));
        }
        let root = Self {
            path: path.into(),
            file,
        };
        root.validate()?;
        Ok(root)
    }

    fn validate(&self) -> Result<(), VmError> {
        let metadata = self.file.metadata().map_err(io_error)?;
        if fs::canonicalize(&self.path).map_err(io_error)? != self.path
            || identity(&open_directory(&self.path)?)? != identity(&self.file)?
            || metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.mode() & 0o022 != 0
        {
            return Err(held("publication observer root was replaced"));
        }
        Ok(())
    }
}

/// Retained root descriptors established before any decorated provision call.
pub(super) struct PhysicalObserver {
    scope: VmProviderOwnerScope,
    runtime: PinnedRoot,
    cgroup: PinnedRoot,
    proc: File,
}

/// The actual pre-start worker, observed without wrapping its returned instance.
#[derive(Clone)]
pub(super) struct WorkerObservation {
    runtime_leaf: Arc<File>,
    cgroup_leaf: Arc<File>,
    pid: u32,
    start_time: u64,
    membership: String,
}

impl PhysicalObserver {
    pub(super) fn new(
        scope: VmProviderOwnerScope,
        runtime: &Path,
        cgroup: &Path,
    ) -> Result<Self, VmError> {
        let observer = Self {
            scope,
            runtime: PinnedRoot::new(runtime)?,
            cgroup: PinnedRoot::new(cgroup)?,
            proc: open_directory(Path::new("/proc"))?,
        };
        // Linux's cgroup-v2 superblock magic rejects emulated directory fixtures.
        if rustix::fs::fstatfs(&observer.cgroup.file)
            .map_err(io_error)?
            .f_type
            != 0x6367_7270
            || rustix::fs::fstatfs(&observer.proc)
                .map_err(io_error)?
                .f_type
                != rustix::fs::PROC_SUPER_MAGIC
        {
            return Err(held(
                "publication observation requires real cgroup v2 and procfs",
            ));
        }
        observer.validate(&observer.scope)?;
        Ok(observer)
    }

    fn validate(&self, scope: &VmProviderOwnerScope) -> Result<(), VmError> {
        if scope != &self.scope {
            return Err(held("publication observer scope differs"));
        }
        self.runtime.validate()?;
        self.cgroup.validate()?;
        if identity(&open_directory(Path::new("/proc"))?)? != identity(&self.proc)? {
            return Err(held("publication procfs observation root changed"));
        }
        Ok(())
    }

    pub(super) fn capture(
        &self,
        scope: &VmProviderOwnerScope,
        id: &VmId,
    ) -> Result<WorkerObservation, VmError> {
        validate_id(id)?;
        self.validate(scope)?;
        let runtime_leaf = Arc::new(open_child(&self.runtime.file, &id.0)?);
        let cgroup_leaf = Arc::new(open_child(&self.cgroup.file, &id.0)?);
        let members = read_text(&cgroup_leaf, "cgroup.procs")?;
        let mut members = members.split_whitespace();
        let pid: u32 = members
            .next()
            .ok_or_else(|| held("publication worker was not observed"))?
            .parse()
            .map_err(io_error)?;
        if pid == 0 || members.next().is_some() {
            return Err(held(
                "publication pre-start cgroup does not contain exactly one worker",
            ));
        }
        let process = open_child(&self.proc, &pid.to_string())?;
        let start_time = process_start(&process, pid)?;
        let membership = read_text(&process, "cgroup")?;
        if !membership.lines().any(|line| line.starts_with("0::/")) {
            return Err(held("publication worker lacks unified cgroup membership"));
        }
        if read_text(&cgroup_leaf, "cgroup.procs")?.trim() != pid.to_string()
            || read_text(&process, "cgroup")? != membership
            || process_start(&process, pid)? != start_time
            || identity(&open_child(&self.cgroup.file, &id.0)?)? != identity(&cgroup_leaf)?
            || identity(&open_child(&self.runtime.file, &id.0)?)? != identity(&runtime_leaf)?
        {
            return Err(held(
                "publication worker or exact leaf changed during observation",
            ));
        }
        self.validate(scope)?;
        Ok(WorkerObservation {
            runtime_leaf,
            cgroup_leaf,
            pid,
            start_time,
            membership,
        })
    }

    pub(super) fn verify(
        &self,
        scope: &VmProviderOwnerScope,
        attempts: &[ProvisionAttempt],
    ) -> Result<Vec<ObservedWorkerExit>, VmError> {
        self.validate(scope)?;
        if attempts.is_empty() {
            return Err(held("empty publication ledger is not physical evidence"));
        }
        let mut exits = Vec::with_capacity(attempts.len());
        for attempt in attempts {
            validate_id(attempt.id())?;
            if attempt.outcome() != ProvisionOutcome::ReturnedHandle || attempt.observation_failed {
                return Err(held(
                    "publication attempt lacks a successful worker observation",
                ));
            }
            let observation = attempt
                .physical
                .as_ref()
                .ok_or_else(|| held("publication worker observation is missing"))?;
            // Retain and read the exact original leaf descriptors even after unlink.
            identity(&observation.runtime_leaf)?;
            identity(&observation.cgroup_leaf)?;
            self.require_unlinked(&observation.runtime_leaf)?;
            self.require_unlinked(&observation.cgroup_leaf)?;
            require_absent(&self.runtime.file, &attempt.id().0)?;
            require_absent(&self.cgroup.file, &attempt.id().0)?;
            let exit = match open_child_raw(&self.proc, &observation.pid.to_string()) {
                Err(rustix::io::Errno::NOENT) => ObservedWorkerExit::Absent {
                    id: attempt.id().clone(),
                    pid: observation.pid,
                },
                Err(error) => return Err(io_error(error)),
                Ok(process) => {
                    let start_time = process_start(&process, observation.pid)?;
                    if start_time == observation.start_time {
                        return Err(held("observed publication worker still exists"));
                    }
                    ObservedWorkerExit::Reused {
                        id: attempt.id().clone(),
                        pid: observation.pid,
                        start_time,
                    }
                }
            };
            if !observation
                .membership
                .lines()
                .any(|line| line.starts_with("0::/"))
            {
                return Err(held("stored publication worker membership is invalid"));
            }
            exits.push(exit);
        }
        self.validate(scope)?;
        Ok(exits)
    }

    fn require_unlinked(&self, file: &File) -> Result<(), VmError> {
        let process = open_child(
            &self.proc,
            &rustix::process::getpid().as_raw_pid().to_string(),
        )?;
        let descriptors = open_child(&process, "fd")?;
        let path = rustix::fs::readlinkat(&descriptors, file.as_raw_fd().to_string(), Vec::new())
            .map_err(io_error)?;
        // Safe VM leaf names cannot end in this kernel-supplied deleted suffix.
        if !path.to_bytes().ends_with(b" (deleted)") {
            return Err(held("observed publication leaf is still linked"));
        }
        Ok(())
    }
}

fn identity(file: &File) -> Result<(u64, u64), VmError> {
    let metadata = file.metadata().map_err(io_error)?;
    Ok((metadata.dev(), metadata.ino()))
}

fn open_directory(path: &Path) -> Result<File, VmError> {
    rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
    .map_err(io_error)
}

fn open_child_raw(root: &File, name: &str) -> Result<File, rustix::io::Errno> {
    rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(File::from)
}

fn open_child(root: &File, name: &str) -> Result<File, VmError> {
    open_child_raw(root, name).map_err(io_error)
}

fn read_text(root: &File, name: &str) -> Result<String, VmError> {
    let file: File = rustix::fs::openat(
        root,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(io_error)?
    .into();
    let mut text = String::new();
    file.take(65_537)
        .read_to_string(&mut text)
        .map_err(io_error)?;
    if text.len() > 65_536 {
        return Err(held("publication observation exceeded its read bound"));
    }
    Ok(text)
}

fn process_start(process: &File, pid: u32) -> Result<u64, VmError> {
    let stat = read_text(process, "stat")?;
    if !stat.starts_with(&format!("{pid} (")) {
        return Err(held("publication process identity differs"));
    }
    // comm can contain spaces and parentheses; starttime is field22, following
    // the final comm delimiter and nineteen preceding fields beginning at state.
    stat.rsplit_once(") ")
        .and_then(|(_, fields)| fields.split_whitespace().nth(19))
        .ok_or_else(|| held("publication process start identity is unavailable"))?
        .parse()
        .map_err(io_error)
}

fn require_absent(root: &File, id: &str) -> Result<(), VmError> {
    match rustix::fs::statat(root, id, AtFlags::SYMLINK_NOFOLLOW) {
        Err(rustix::io::Errno::NOENT) => Ok(()),
        Err(error) => Err(io_error(error)),
        Ok(_) => Err(held("publication runtime or cgroup leaf remains")),
    }
}

fn validate_id(id: &VmId) -> Result<(), VmError> {
    if id.0.is_empty()
        || id.0.len() > 64
        || !id
            .0
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(held("publication observation VM identifier is unsafe"));
    }
    Ok(())
}

const fn held(reason: &'static str) -> VmError {
    VmError::InvalidState(reason)
}

fn io_error(error: impl std::error::Error + Send + Sync + 'static) -> VmError {
    VmError::Provider {
        provider: "libkrun".into(),
        code: "publication_physical_observation".into(),
        source: Box::new(error),
    }
}
