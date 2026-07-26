use rustix::process::{getegid, geteuid};
use serde::{Deserialize, Serialize};
use std::{ffi::OsString, path::PathBuf, time::Duration};

/// Per-device cgroup-v2 I/O throttling limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IoLimit {
    /// Linux block-device major number.
    pub major: u32,
    /// Linux block-device minor number.
    pub minor: u32,
    /// Maximum read bytes per second.
    pub read_bps: Option<u64>,
    /// Maximum write bytes per second.
    pub write_bps: Option<u64>,
    /// Maximum read operations per second.
    pub read_iops: Option<u64>,
    /// Maximum write operations per second.
    pub write_iops: Option<u64>,
}

/// Resource limits applied to one worker's delegated cgroup.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CgroupLimits {
    /// CPU quota in microseconds per period, or no quota.
    pub cpu_quota_micros: Option<u64>,
    /// CPU quota period in microseconds.
    pub cpu_period_micros: u64,
    /// Maximum resident memory for the worker and descendants.
    pub memory_max_bytes: u64,
    /// Maximum worker and guest-helper process count.
    pub pids_max: u32,
    /// Per-device cgroup-v2 I/O throttles.
    pub io: Vec<IoLimit>,
    /// Maximum aggregate size of writable disk images in the VM specification.
    pub writable_disk_max_bytes: u64,
    /// Maximum VM wall-clock lifetime.
    pub wall_clock_timeout: Duration,
}

impl Default for CgroupLimits {
    fn default() -> Self {
        Self {
            cpu_quota_micros: None,
            cpu_period_micros: 100_000,
            memory_max_bytes: 2 * 1024 * 1024 * 1024,
            pids_max: 512,
            io: Vec::new(),
            writable_disk_max_bytes: 20 * 1024 * 1024 * 1024,
            wall_clock_timeout: Duration::from_secs(60 * 60),
        }
    }
}

/// Host configuration for the Fedora/Linux libkrun provider.
#[derive(Debug, Clone)]
pub struct LibkrunConfig {
    /// Parent directory for private per-VM runtime directories.
    pub runtime_root: PathBuf,
    /// Canonical roots from which root filesystems may be selected.
    pub image_roots: Vec<PathBuf>,
    /// Canonical roots from which block disks may be selected.
    pub disk_roots: Vec<PathBuf>,
    /// Canonical roots from which virtio-fs mounts may be selected.
    pub mount_roots: Vec<PathBuf>,
    /// Dedicated worker executable.
    pub worker_binary: PathBuf,
    /// `passt` executable.
    pub passt_binary: PathBuf,
    /// KVM device checked before provisioning.
    pub kvm_device: PathBuf,
    /// libkrun 1.x shared object name or absolute path.
    pub libkrun_library: OsString,
    /// Delegated cgroup-v2 subtree owned by the service account.
    pub cgroup_root: PathBuf,
    /// Require `cgroup_root` to be a real cgroup-v2 filesystem.
    ///
    /// This should remain enabled outside unit tests.
    pub enforce_cgroup_v2: bool,
    /// Required effective service UID.
    pub service_uid: u32,
    /// Required effective service GID.
    pub service_gid: u32,
    /// Maximum time to configure and connect a worker.
    pub startup_timeout: Duration,
    /// Maximum time to wait for `heph-init` readiness.
    pub readiness_timeout: Duration,
    /// Per-VM resource limits.
    pub limits: CgroupLimits,
}

impl LibkrunConfig {
    /// Creates configuration with secure Linux defaults for executable and
    /// device locations.
    #[must_use]
    pub fn new(
        runtime_root: impl Into<PathBuf>,
        image_roots: Vec<PathBuf>,
        disk_roots: Vec<PathBuf>,
        mount_roots: Vec<PathBuf>,
        worker_binary: impl Into<PathBuf>,
        cgroup_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            runtime_root: runtime_root.into(),
            image_roots,
            disk_roots,
            mount_roots,
            worker_binary: worker_binary.into(),
            passt_binary: PathBuf::from("/usr/bin/passt"),
            kvm_device: PathBuf::from("/dev/kvm"),
            libkrun_library: OsString::from("libkrun.so.1"),
            cgroup_root: cgroup_root.into(),
            enforce_cgroup_v2: true,
            service_uid: geteuid().as_raw(),
            service_gid: getegid().as_raw(),
            startup_timeout: Duration::from_secs(10),
            readiness_timeout: Duration::from_secs(30),
            limits: CgroupLimits::default(),
        }
    }
}
