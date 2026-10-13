use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use tempfile::TempDir;
use uuid::Uuid;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    DiskFormat, GuestCommand, GuestMountPath, NetworkMode, RootFilesystem, VmDisk, VmGuestVolume,
    VmId, VmResources, VmSpec, VolumeAccessMode,
};

pub struct Fixture {
    pub directory: TempDir,
    pub provider: LibkrunProvider,
    pub read_only: PathBuf,
    pub writable: PathBuf,
    rootfs: PathBuf,
    cgroup_root: PathBuf,
    facts_uuid: Uuid,
    data_uuid: Uuid,
}

impl Fixture {
    pub fn new() -> Self {
        let directory = tempfile::tempdir_in("/tmp").expect("private native fixtures");
        fs::create_dir(directory.path().join("runtime")).unwrap();
        let rootfs = PathBuf::from(std::env::var("HEPHAESTUS_LIBKRUN_NAMED_ROOTFS").unwrap());
        let cgroup_root =
            PathBuf::from(std::env::var("HEPHAESTUS_LIBKRUN_NAMED_CGROUP_ROOT").unwrap());
        let facts_uuid = Uuid::new_v4();
        let data_uuid = Uuid::new_v4();
        let read_only = create_image(directory.path(), "facts", facts_uuid, "immutable-facts\n");
        let writable = create_image(directory.path(), "data", data_uuid, "mutable-data\n");
        let mut config = LibkrunConfig::new(
            directory.path().join("runtime"),
            vec![rootfs.clone()],
            vec![directory.path().to_path_buf()],
            vec![directory.path().to_path_buf()],
            env!("CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"),
            cgroup_root.clone(),
        );
        config.startup_timeout = Duration::from_secs(20);
        config.readiness_timeout = Duration::from_secs(30);
        println!(
            "NAMED_VOLUME_WORKER=1 executable_sha256={}",
            hash(Path::new(env!(
                "CARGO_BIN_EXE_hephaestus-vm-libkrun-worker"
            )))
        );
        let provider = LibkrunProvider::new(config).expect("real native host prerequisites");
        Self {
            directory,
            provider,
            read_only,
            writable,
            rootfs,
            cgroup_root,
            facts_uuid,
            data_uuid,
        }
    }

    pub fn spec(&self, id: &str) -> VmSpec {
        let guest_volumes = [
            (
                "facts",
                self.facts_uuid,
                "/volumes/ro",
                VolumeAccessMode::ReadOnly,
            ),
            (
                "data",
                self.data_uuid,
                "/volumes/rw",
                VolumeAccessMode::ReadWrite,
            ),
        ]
        .into_iter()
        .map(|(slot, uuid, path, mode)| {
            VmGuestVolume::new(
                serde_json::from_value(serde_json::json!(slot)).unwrap(),
                slot,
                uuid,
                GuestMountPath::parse(path).unwrap(),
                mode,
            )
            .unwrap()
        })
        .collect();
        VmSpec {
            id: VmId(id.to_owned()),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
            guest_volumes,
            disks: vec![
                VmDisk {
                    id: "facts".to_owned(),
                    host_path: self.read_only.clone(),
                    format: DiskFormat::Raw,
                    read_only: true,
                },
                VmDisk {
                    id: "data".to_owned(),
                    host_path: self.writable.clone(),
                    format: DiskFormat::Raw,
                    read_only: false,
                },
            ],
            mounts: vec![],
            resources: VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            network: NetworkMode::Disabled,
            // Only the disposable test root contains this native probe at the
            // trusted builder path. Positive controls require real guest root.
            command: GuestCommand {
                program: "/usr/libexec/hephaestus/oci-build".to_owned(),
                args: vec!["--named-volumes-root-proof".to_owned()],
                env: BTreeMap::from([
                    ("HEPH_PLATFORM_OCI_BUILDER".to_owned(), "1".to_owned()),
                    ("HEPH_TEST_RO_UUID".to_owned(), self.facts_uuid.to_string()),
                    ("HEPH_TEST_RW_UUID".to_owned(), self.data_uuid.to_string()),
                ]),
                working_dir: Some(PathBuf::from("/")),
            },
            runtime_authority: None,
            private_http_service: None,
            runtime_git_bridge: None,
            labels: BTreeMap::new(),
        }
    }

    pub fn assert_clean(&self, id: &str) {
        assert!(!self.directory.path().join("runtime").join(id).exists());
        assert!(!self.cgroup_root.join(id).exists());
    }
}

fn create_image(root: &Path, name: &str, uuid: Uuid, sentinel: &str) -> PathBuf {
    let tree = root.join(name);
    fs::create_dir(&tree).unwrap();
    fs::write(tree.join("sentinel"), sentinel).unwrap();
    let path = root.join(format!("{name}.raw"));
    File::create(&path)
        .unwrap()
        .set_len(64 * 1024 * 1024)
        .unwrap();
    let result = Command::new("/usr/sbin/mkfs.ext4")
        .args(["-q", "-F", "-U"])
        .arg(uuid.to_string())
        .arg("-d")
        .arg(tree)
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "mkfs: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    path
}

pub fn hash(path: &Path) -> String {
    let result = Command::new("/usr/bin/sha256sum")
        .arg(path)
        .output()
        .unwrap();
    assert!(result.status.success());
    String::from_utf8(result.stdout)
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_owned()
}

pub fn read_persisted(path: &Path) -> String {
    read_data_file(path, "cat /persisted")
}

pub fn read_data_file(path: &Path, request: &str) -> String {
    let result = Command::new("/usr/sbin/debugfs")
        .args(["-R", request])
        .arg(path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "debugfs: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap()
}
