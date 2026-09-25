use crate::config::LibkrunConfig;
use std::{collections::BTreeMap, fs, path::PathBuf};
use tempfile::TempDir;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmId, VmResources, VmSpec};

pub(super) fn assert_invalid_field(
    result: Result<super::super::PreparedSpec, vm_trait::VmError>,
    expected: &str,
) {
    assert!(matches!(
        result,
        Err(vm_trait::VmError::InvalidSpec { field, .. }) if field.starts_with(expected)
    ));
}

pub(super) struct Fixture {
    pub(super) temp: TempDir,
    pub(super) images: PathBuf,
    pub(super) disks: PathBuf,
    pub(super) mounts: PathBuf,
    pub(super) config: LibkrunConfig,
}

impl Fixture {
    pub(super) fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let images = temp.path().join("images");
        let disks = temp.path().join("disks");
        let mounts = temp.path().join("mounts");
        for directory in [&images, &disks, &mounts] {
            fs::create_dir(directory).unwrap();
        }
        let root = images.join("root");
        fs::create_dir(&root).unwrap();
        let config = LibkrunConfig::new(
            temp.path(),
            vec![images.clone()],
            vec![disks.clone()],
            vec![mounts.clone()],
            "/bin/true",
            temp.path(),
        );
        Self {
            temp,
            images,
            disks,
            mounts,
            config,
        }
    }

    pub(super) fn spec(&self) -> VmSpec {
        VmSpec {
            id: VmId("validation".to_owned()),
            root: RootFilesystem::Directory {
                host_path: self.images.join("root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 256,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: "/bin/true".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from("/")),
            },
            runtime_authority: None,
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::new(),
        }
    }

    pub(super) fn valid_config(&self) -> LibkrunConfig {
        let kvm = self.temp.path().join("kvm");
        fs::write(&kvm, []).unwrap();
        let mut config = self.config.clone();
        config.passt_binary = PathBuf::from("/bin/true");
        config.kvm_device = kvm;
        config.enforce_cgroup_v2 = false;
        config
    }
}
