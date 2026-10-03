use super::images::Image;
use std::{collections::BTreeMap, fs, path::PathBuf, time::Duration};
use tempfile::TempDir;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    DiskFormat, GuestCommand, GuestMountPath, NetworkMode, RootFilesystem, VmDisk, VmGuestVolume,
    VmId, VmResources, VmSpec, VmVolumeInitializationPurpose, VolumeAccessMode,
};

pub struct Fixture {
    directory: Option<TempDir>,
    pub provider: LibkrunProvider,
    pub state: Image,
    pub seeded: Image,
    pub data: Image,
    pub facts: Image,
    rootfs: PathBuf,
    cgroup_root: PathBuf,
    verified: bool,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir_in("/tmp").unwrap();
        let rootfs = PathBuf::from(
            std::env::var_os("HEPHAESTUS_LIBKRUN_INITIALIZATION_ROOTFS")
                .expect("fresh protocol11 root"),
        );
        let cgroup_root = PathBuf::from(
            std::env::var_os("HEPHAESTUS_LIBKRUN_INITIALIZATION_CGROUP_ROOT")
                .expect("dedicated fresh delegated cgroup"),
        );
        let runtime_root = directory.path().join("runtime");
        fs::create_dir(&runtime_root).unwrap();
        let mut config = LibkrunConfig::new(
            runtime_root,
            vec![rootfs.clone()],
            vec![directory.path().to_owned()],
            vec![directory.path().to_owned()],
            env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
            cgroup_root.clone(),
        );
        config.startup_timeout = Duration::from_secs(20);
        config.readiness_timeout = Duration::from_secs(30);
        let provider = LibkrunProvider::new_owned(config, "protocol11-native-host")
            .expect("fresh owned native provider");
        let state = Image::new(directory.path(), "state");
        let seeded = Image::new(directory.path(), "seeded");
        let data = Image::new(directory.path(), "data");
        let facts = Image::new(directory.path(), "facts");
        Self {
            directory: Some(directory),
            provider,
            state,
            seeded,
            data,
            facts,
            rootfs,
            cgroup_root,
            verified: false,
        }
    }

    pub fn spec(&self, mode: &str) -> VmSpec {
        let state = if mode == "seeded" {
            &self.seeded
        } else {
            &self.state
        };
        let selections = [
            (
                "state",
                state,
                "/var/lib/hephaestus",
                VolumeAccessMode::ReadWrite,
                VmVolumeInitializationPurpose::BuiltinStateSQLite,
            ),
            (
                "data",
                &self.data,
                "/data",
                VolumeAccessMode::ReadWrite,
                VmVolumeInitializationPurpose::None,
            ),
            (
                "facts",
                &self.facts,
                "/facts",
                VolumeAccessMode::ReadOnly,
                VmVolumeInitializationPurpose::None,
            ),
        ];
        let guest_volumes = selections
            .iter()
            .map(|(slot, image, path, access, purpose)| {
                VmGuestVolume::new(
                    serde_json::from_value(serde_json::json!(slot)).unwrap(),
                    *slot,
                    image.uuid,
                    GuestMountPath::parse(*path).unwrap(),
                    *access,
                )
                .unwrap()
                .with_initialization_purpose(
                    *purpose,
                    *purpose == VmVolumeInitializationPurpose::BuiltinStateSQLite,
                )
                .unwrap()
            })
            .collect();
        let disks = selections
            .iter()
            .map(|(slot, image, _, access, _)| VmDisk {
                id: (*slot).to_owned(),
                host_path: image.path.clone(),
                format: DiskFormat::Raw,
                read_only: *access == VolumeAccessMode::ReadOnly,
            })
            .collect();
        VmSpec {
            id: VmId(format!("protocol11-{mode}")),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
            disks,
            mounts: vec![],
            guest_volumes,
            resources: VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            network: NetworkMode::Disabled,
            // This path has no trusted root-operation classification or flag.
            command: GuestCommand {
                program: "/usr/libexec/hephaestus/integration-check".to_owned(),
                args: vec!["--guest-initialization-proof".to_owned(), mode.to_owned()],
                env: BTreeMap::new(),
                working_dir: Some("/".into()),
            },
            runtime_authority: None,
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::new(),
        }
    }

    pub fn worker_pids(&self, mode: &str) -> Vec<u32> {
        // Query only this exact VM's known cgroup, never scan host processes.
        let path = self
            .cgroup_root
            .join(format!("protocol11-{mode}"))
            .join("cgroup.procs");
        let mut pids = fs::read_to_string(path)
            .expect("known live VM cgroup")
            .lines()
            .map(|line| line.parse::<u32>().expect("cgroup process ID"))
            .collect::<Vec<_>>();
        pids.sort_unstable();
        pids.dedup();
        assert!(
            !pids.is_empty(),
            "provisioned VM must have a known worker PID"
        );
        for pid in &pids {
            assert!(*pid > 0 && PathBuf::from(format!("/proc/{pid}")).exists());
        }
        println!("GUEST_INITIALIZATION_PIDS=1 mode={mode} pids={pids:?}");
        pids
    }

    pub fn assert_clean(&self, mode: &str, pids: &[u32]) {
        let id = format!("protocol11-{mode}");
        assert!(
            !self
                .directory
                .as_ref()
                .unwrap()
                .path()
                .join("runtime")
                .join(&id)
                .exists()
        );
        assert!(!self.cgroup_root.join(id).exists());
        for pid in pids {
            assert!(
                !PathBuf::from(format!("/proc/{pid}")).exists(),
                "known VM worker {pid} remains after destruction"
            );
        }
        println!(
            "GUEST_INITIALIZATION_DESTROYED=1 mode={mode} known_pids_absent=1 runtime_absent=1 cgroup_absent=1"
        );
    }

    pub const fn mark_verified(&mut self) {
        self.verified = true;
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if !self.verified
            && let Some(directory) = self.directory.take()
        {
            eprintln!(
                "Retaining unverified native initialization evidence: {}",
                directory.keep().display()
            );
        }
    }
}
