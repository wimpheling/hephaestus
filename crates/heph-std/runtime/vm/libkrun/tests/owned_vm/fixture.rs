use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::Duration,
};
use tempfile::TempDir;
use vm_libkrun::LibkrunConfig;
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, VmId, VmProviderOwnerScope, VmResources, VmSpec,
};

pub const HOST: &str = "owned-native-host";

pub struct Fixture {
    pub directory: TempDir,
    pub config: LibkrunConfig,
    rootfs: PathBuf,
}

impl Fixture {
    pub fn new() -> Self {
        assert_ne!(
            rustix::process::geteuid().as_raw(),
            0,
            "native provider runs without root"
        );
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        fs::create_dir(directory.path().join("runtime")).unwrap();
        Self::from_directory(directory)
    }

    pub fn child() -> Self {
        // This disposable handle does not own the parent's persistent roots.
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let mut fixture = Self::from_directory(directory);
        fixture.config.runtime_root = std::env::var_os("HEPH_OWNED_VM_CHILD_RUNTIME")
            .unwrap()
            .into();
        fixture
    }

    fn from_directory(directory: TempDir) -> Self {
        let rootfs = PathBuf::from(
            std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_ROOTFS").expect("fresh native root"),
        );
        let cgroup = PathBuf::from(
            std::env::var_os("HEPHAESTUS_LIBKRUN_OWNED_CGROUP_ROOT")
                .expect("dedicated delegated cgroup"),
        );
        let mut config = LibkrunConfig::new(
            directory.path().join("runtime"),
            vec![rootfs.clone()],
            vec![rootfs.clone()],
            vec![rootfs.clone()],
            env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
            cgroup,
        );
        config.startup_timeout = Duration::from_secs(20);
        config.readiness_timeout = Duration::from_secs(30);
        Self {
            directory,
            config,
            rootfs,
        }
    }

    pub fn spec(&self, id: &str) -> VmSpec {
        VmSpec {
            id: VmId(id.into()),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            guest_volumes: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: "/bin/sleep".into(),
                args: vec!["300".into()],
                env: BTreeMap::new(),
                working_dir: Some("/".into()),
            },
            runtime_authority: None,
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::new(),
        }
    }

    pub fn assert_marker(&self, scope: &VmProviderOwnerScope) {
        let marker: serde_json::Value = serde_json::from_slice(
            &fs::read(self.config.runtime_root.join(".heph-vm-owner.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(marker["version"], 2);
        assert_eq!(marker["namespace"], scope.namespace());
        println!(
            "OWNED_VM_PHYSICAL_ROOT=1 path={} device={} inode={} cgroup_root={} marker_version=2 worker_sha256={}",
            self.config.runtime_root.display(),
            fs::metadata(&self.config.runtime_root).unwrap().dev(),
            fs::metadata(&self.config.runtime_root).unwrap().ino(),
            self.config.cgroup_root.display(),
            hash(Path::new(env!(
                "CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"
            )))
        );
    }

    pub fn assert_live(&self, id: &VmId) {
        let cgroup = self.config.cgroup_root.join(&id.0);
        let pids = fs::read_to_string(cgroup.join("cgroup.procs")).unwrap();
        assert!(!pids.trim().is_empty(), "actual KVM worker in real cgroup");
        for pid in pids.split_whitespace() {
            assert!(Path::new("/proc").join(pid).exists());
        }
        assert!(self.config.runtime_root.join(&id.0).is_dir());
        assert_eq!(
            fs::read_to_string(cgroup.join("memory.max"))
                .unwrap()
                .trim(),
            self.config.limits.memory_max_bytes.to_string()
        );
        println!(
            "OWNED_VM_LIVE_KVM=1 id={} worker_pids={} cgroup={}",
            id.0,
            pids.trim().replace('\n', ","),
            cgroup.display()
        );
    }

    pub fn assert_clean(&self, id: &VmId) {
        assert!(!self.config.runtime_root.join(&id.0).exists());
        assert!(!self.config.cgroup_root.join(&id.0).exists());
    }
}

pub fn hash(path: &Path) -> String {
    let output = std::process::Command::new("sha256sum")
        .arg(path)
        .output()
        .unwrap();
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .into()
}

pub fn entries(root: &Path) -> Vec<(String, u64, Vec<u8>)> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            let entry = entry.unwrap();
            (
                entry.file_name().to_string_lossy().into_owned(),
                entry.metadata().unwrap().ino(),
                fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    entries.sort();
    entries
}
