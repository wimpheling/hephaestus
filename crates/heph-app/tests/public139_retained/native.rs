use std::{path::PathBuf, sync::Arc, time::Duration};
use tempfile::TempDir;
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
use volume_postgres::PostgresVolumeMetadataRepository;

pub struct Native {
    pub owner: Arc<VolumeRootOwner>,
    pub local: Arc<LocalVolumeStore>,
    pub provider: Arc<LibkrunProvider>,
    pub config: LocalVolumeConfig,
    // Owners and stores drop before the temporary directory removes their roots.
    _root: TempDir,
}

impl Native {
    pub fn new(
        actor: Arc<PostgresVolumeMetadataRepository>,
        worker: Arc<PostgresVolumeMetadataRepository>,
    ) -> Self {
        let root = TempDir::new().unwrap();
        let runtime = root.path().join("vm-runtime");
        let volumes = root.path().join("volumes");
        let mounts = root.path().join("mounts");
        for path in [&runtime, &volumes, &mounts] {
            std::fs::create_dir(path).unwrap();
        }
        let provider = Arc::new(
            LibkrunProvider::new_owned(
                LibkrunConfig::new(
                    runtime.clone(),
                    vec![required("HEPHAESTUS_LIBKRUN_IMAGE_ROOT")],
                    vec![volumes.clone()],
                    vec![mounts.clone()],
                    required("HEPHAESTUS_LIBKRUN_WORKER"),
                    required("HEPHAESTUS_LIBKRUN_CGROUP_ROOT"),
                ),
                "public139-retained-native",
            )
            .unwrap(),
        );
        // Real defaults retain service UID/GID, /dev/kvm, passt and cgroup-v2 checks.
        let owner =
            Arc::new(VolumeRootOwner::initialize(&volumes, "public139-retained-native").unwrap());
        let config = LocalVolumeConfig {
            volume_root: volumes,
            transient_runtime_roots: vec![runtime, mounts],
            host_id: "public139-retained-native".into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: required("HEPHAESTUS_PUBLIC139_MKFS_EXT4"),
        };
        let local = Arc::new(
            LocalVolumeStore::new(worker.clone(), config.clone())
                .unwrap()
                .with_owned_metadata(OwnedVolumeMetadata::new(actor, worker, owner.clone()))
                .unwrap(),
        );
        Self {
            _root: root,
            owner,
            local,
            provider,
            config,
        }
    }
}

fn required(name: &str) -> PathBuf {
    std::env::var_os(name).map_or_else(
        || panic!("native ignored test requires actual configured {name}"),
        PathBuf::from,
    )
}
