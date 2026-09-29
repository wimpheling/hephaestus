use std::{collections::BTreeMap, env, fs, path::PathBuf};

use crate::ENABLE_FLAG;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmId, VmResources, VmSpec};

pub fn sqlite_previous_rows(markers: &str) -> u64 {
    markers
        .lines()
        .find_map(|line| line.strip_prefix("sqlite_previous="))
        .expect("missing sqlite_previous marker")
        .parse()
        .expect("invalid sqlite_previous marker")
}

pub fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}

pub fn required_text(name: &str) -> String {
    env::var(name).unwrap_or_else(|_| panic!("{name} must be set when {ENABLE_FLAG}=1"))
}

pub fn assert_cgroup_limits(path: &std::path::Path, limits: &vm_libkrun::CgroupLimits) {
    let cpu = limits.cpu_quota_micros.map_or_else(
        || format!("max {}", limits.cpu_period_micros),
        |quota| format!("{quota} {}", limits.cpu_period_micros),
    );
    assert_eq!(
        fs::read_to_string(path.join("cpu.max"))
            .expect("read worker CPU limit")
            .trim(),
        cpu
    );
    assert_eq!(
        fs::read_to_string(path.join("memory.max"))
            .expect("read worker memory limit")
            .trim(),
        limits.memory_max_bytes.to_string()
    );
    assert_eq!(
        fs::read_to_string(path.join("pids.max"))
            .expect("read worker PID limit")
            .trim(),
        limits.pids_max.to_string()
    );
    assert!(
        !fs::read_to_string(path.join("cgroup.procs"))
            .expect("read worker cgroup membership")
            .trim()
            .is_empty(),
        "worker cgroup contains no processes"
    );
}

pub fn long_running_spec(rootfs: PathBuf, kind: &str) -> VmSpec {
    VmSpec {
        id: VmId(format!("integration-{kind}-{}", std::process::id())),
        root: RootFilesystem::Directory { host_path: rootfs },
        disks: Vec::new(),
        mounts: Vec::new(),
        resources: VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        network: NetworkMode::Disabled,
        command: GuestCommand {
            program: "/bin/sleep".to_owned(),
            args: vec!["300".to_owned()],
            env: BTreeMap::new(),
            working_dir: Some(PathBuf::from("/")),
        },
        runtime_authority: None,
        private_http_service: None,
        runtime_git_bridge: None,
        labels: BTreeMap::from([("test".to_owned(), kind.to_owned())]),
    }
}
