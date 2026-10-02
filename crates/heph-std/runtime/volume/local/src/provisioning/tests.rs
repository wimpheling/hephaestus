use super::{open_existing, open_root, prove_ext4};
use crate::{LocalVolumeConfig, LocalVolumeStore};
use async_trait::async_trait;
use runtime_types::{AgentInstanceId, RunId, VolumeId};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    sync::{Arc, Mutex},
    time::Duration,
};
use tempfile::TempDir;
use time::OffsetDateTime;
use uuid::Uuid;
use volume_trait::{
    ProvisioningClaim, Volume, VolumeError, VolumeKind, VolumeLease, VolumeMetadataRepository,
    VolumeProvisioningState, VolumeState,
};

fn file_with_superblock(root: &TempDir, id: Uuid, blocks: u32) -> fs::File {
    let path = root.path().join("probe.raw");
    let mut bytes = vec![0_u8; 16 * 1024 * 1024];
    bytes[1024 + 56..1024 + 58].copy_from_slice(&[0x53, 0xef]);
    bytes[1024 + 104..1024 + 120].copy_from_slice(id.as_bytes());
    bytes[1024 + 4..1024 + 8].copy_from_slice(&blocks.to_le_bytes());
    bytes[1024 + 96..1024 + 100].copy_from_slice(&0x40_u32.to_le_bytes());
    fs::write(&path, bytes).expect("write probe");
    fs::File::open(path).expect("open probe")
}

#[test]
fn filesystem_proof_requires_exact_uuid_file_length_and_block_geometry() {
    let root = TempDir::new().expect("probe root");
    let id = Uuid::new_v4();
    let file = file_with_superblock(&root, id, 16384);
    assert!(prove_ext4(&file, 16 * 1024 * 1024, id).is_ok());
    assert!(prove_ext4(&file, 16 * 1024 * 1024, Uuid::new_v4()).is_err());
    assert!(prove_ext4(&file, 32 * 1024 * 1024, id).is_err());
    let file = file_with_superblock(&root, id, 8192);
    assert!(prove_ext4(&file, 16 * 1024 * 1024, id).is_err());
}

#[test]
fn descriptor_open_rejects_symlink_without_touching_target() {
    let root = TempDir::new().expect("volume root");
    let outside = TempDir::new().expect("outside root");
    let target = outside.path().join("retained");
    fs::write(&target, b"retained bytes").expect("target");
    symlink(&target, root.path().join("volume.raw")).expect("symlink");
    let directory = open_root(root.path()).expect("open root");
    assert!(open_existing(&directory, "volume.raw").is_err());
    assert_eq!(fs::read(target).expect("read target"), b"retained bytes");
}

struct MemoryMetadata {
    volume: Mutex<Volume>,
    fail_ready_once: Mutex<bool>,
}

fn unsupported<T>() -> Result<T, VolumeError> {
    Err(VolumeError::InvalidState(
        "not used by provisioning fixture",
    ))
}

#[async_trait]
impl VolumeMetadataRepository for MemoryMetadata {
    async fn resolve_instance_state(
        &self,
        _: AgentInstanceId,
        _: u64,
        _: &str,
        _: &std::path::Path,
        _: Uuid,
    ) -> Result<Volume, VolumeError> {
        self.volume(VolumeId::new()).await
    }
    async fn reserve_provider(
        &self,
        _: VolumeId,
        _: &str,
        _: &std::path::Path,
    ) -> Result<Volume, VolumeError> {
        self.volume(VolumeId::new()).await
    }
    async fn volume(&self, _: VolumeId) -> Result<Volume, VolumeError> {
        Ok(self.volume.lock().expect("volume lock").clone())
    }
    async fn claim_provisioning(&self, _: VolumeId) -> Result<ProvisioningClaim, VolumeError> {
        let mut volume = self.volume.lock().expect("volume lock");
        volume.provisioning_generation += 1;
        Ok(ProvisioningClaim {
            volume: volume.clone(),
        })
    }
    async fn provisioning_progress(
        &self,
        claim: &ProvisioningClaim,
        state: VolumeProvisioningState,
    ) -> Result<(), VolumeError> {
        let mut volume = self.volume.lock().expect("volume lock");
        if volume.provisioning_generation != claim.volume.provisioning_generation {
            return Err(VolumeError::StaleLease);
        }
        if state == VolumeProvisioningState::Ready {
            let mut fail = self.fail_ready_once.lock().expect("failure lock");
            let should_fail = *fail;
            *fail = false;
            drop(fail);
            if should_fail {
                return Err(VolumeError::InvalidState(
                    "simulated readiness commit failure",
                ));
            }
            volume.state = VolumeState::Ready;
        }
        volume.provisioning_state = state;
        drop(volume);
        Ok(())
    }
    async fn mark_ready(&self, _: VolumeId) -> Result<(), VolumeError> {
        unsupported()
    }
    async fn acquire(
        &self,
        _: VolumeId,
        _: RunId,
        _: &str,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        unsupported()
    }
    async fn mark_attached(
        &self,
        _: &VolumeLease,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        unsupported()
    }
    async fn heartbeat(
        &self,
        _: &VolumeLease,
        _: OffsetDateTime,
        _: OffsetDateTime,
    ) -> Result<VolumeLease, VolumeError> {
        unsupported()
    }
    async fn active_lease_for_run(
        &self,
        _: RunId,
        _: &str,
    ) -> Result<Option<VolumeLease>, VolumeError> {
        unsupported()
    }
    async fn release_after_detach(&self, _: &VolumeLease, _: bool) -> Result<(), VolumeError> {
        unsupported()
    }
    async fn stale_leases(&self, _: OffsetDateTime) -> Result<Vec<VolumeLease>, VolumeError> {
        unsupported()
    }
    async fn begin_recovery(&self, _: &VolumeLease) -> Result<(), VolumeError> {
        unsupported()
    }
}

fn fixture(
    root: &TempDir,
    fail_ready_once: bool,
    mkfs: std::path::PathBuf,
) -> (LocalVolumeStore, Arc<MemoryMetadata>, VolumeId) {
    let id = VolumeId::new();
    let metadata = Arc::new(MemoryMetadata {
        volume: Mutex::new(Volume {
            id,
            project_id: Uuid::new_v4(),
            instance_id: None,
            kind: VolumeKind::Private,
            host_id: "fixture-host".into(),
            host_path: root.path().join(format!("{id}.raw")),
            capacity_bytes: 16 * 1024 * 1024,
            filesystem_uuid: Uuid::new_v4(),
            state: VolumeState::Uninitialized,
            provisioning_state: VolumeProvisioningState::Reserved,
            provisioning_generation: 0,
            key_reference: None,
            encryption_version: None,
            backup_revision: None,
            checksum: None,
            last_successful_backup_at: None,
        }),
        fail_ready_once: Mutex::new(fail_ready_once),
    });
    let store = LocalVolumeStore::new(
        metadata.clone(),
        LocalVolumeConfig {
            volume_root: root.path().to_owned(),
            transient_runtime_roots: Vec::new(),
            host_id: "fixture-host".into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: mkfs,
        },
    )
    .expect("store");
    (store, metadata, id)
}

#[tokio::test]
async fn unknown_existing_bytes_are_never_truncated_or_formatted() {
    let root = TempDir::new().expect("root");
    let (store, metadata, id) = fixture(&root, false, "/not/executed/mkfs.ext4".into());
    let path = metadata.volume.lock().expect("volume").host_path.clone();
    fs::write(&path, b"unknown existing bytes").expect("existing file");
    assert!(store.provision(id).await.is_err());
    assert_eq!(
        fs::read(path).expect("retained bytes"),
        b"unknown existing bytes"
    );
    assert_eq!(
        metadata.volume.lock().expect("volume").provisioning_state,
        VolumeProvisioningState::Uncertain
    );
}

#[tokio::test]
async fn ambiguous_partial_creation_is_retained_on_retry() {
    let root = TempDir::new().expect("root");
    let (store, metadata, id) = fixture(&root, false, "/not/executed/mkfs.ext4".into());
    let path = metadata.volume.lock().expect("volume").host_path.clone();
    fs::write(&path, vec![0_u8; 4096]).expect("partial allocation");
    metadata.volume.lock().expect("volume").provisioning_state = VolumeProvisioningState::Creating;
    assert!(store.provision(id).await.is_err());
    assert_eq!(
        fs::metadata(path).expect("retained partial file").len(),
        4096
    );
}

#[tokio::test]
async fn native_ext4_is_adopted_after_failed_readiness_commit_without_reformatting() {
    let root = TempDir::new().expect("root");
    let tools = TempDir::new().expect("fixture tools");
    let real_mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .expect("install e2fsprogs for native volume provisioning tests");
    let real_fsck = std::path::Path::new(real_mkfs)
        .parent()
        .expect("parent")
        .join("e2fsck");
    assert!(
        real_fsck.is_file(),
        "install e2fsprogs for readonly filesystem proof"
    );
    let mkfs = tools.path().join("mkfs.ext4");
    let count = tools.path().join("formats");
    fs::write(
        &mkfs,
        format!(
            "#!/bin/sh\nprintf 'format\\n' >> '{}'\nexec '{}' \"$@\"\n",
            count.display(),
            real_mkfs
        ),
    )
    .expect("mkfs wrapper");
    fs::set_permissions(&mkfs, fs::Permissions::from_mode(0o700)).expect("wrapper permission");
    symlink(real_fsck, tools.path().join("e2fsck")).expect("fsck fixture");
    let (store, metadata, id) = fixture(&root, true, mkfs);
    assert!(
        store.provision(id).await.is_err(),
        "first readiness write fails"
    );
    let path = metadata.volume.lock().expect("volume").host_path.clone();
    let before = fs::read(&path).expect("formatted retained bytes");
    let ready = store
        .provision(id)
        .await
        .expect("adopt proven ext4 after crash");
    assert_eq!(ready.state, VolumeState::Ready);
    assert_eq!(ready.provisioning_state, VolumeProvisioningState::Ready);
    assert_eq!(fs::read(path).expect("unchanged bytes"), before);
    assert_eq!(fs::read_to_string(count).expect("format count"), "format\n");
}

#[tokio::test]
async fn subprocess_retains_exclusion_after_parent_lock_descriptor_is_dropped() {
    let root = TempDir::new().expect("root");
    let directory = open_root(root.path()).expect("root descriptor");
    let id = VolumeId::new();
    let lock = super::open_lock(&directory, id).expect("lock descriptor");
    rustix::fs::flock(&lock, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .expect("exclusive lock");
    let mut child = super::locked_command(std::path::Path::new("/bin/sleep"), &lock)
        .expect("locked command")
        .arg("60")
        .spawn()
        .expect("start child");
    drop(lock);
    let competing = super::open_lock(&directory, id).expect("competing descriptor");
    assert!(
        rustix::fs::flock(
            &competing,
            rustix::fs::FlockOperation::NonBlockingLockExclusive
        )
        .is_err(),
        "running formatter keeps exclusion after parent cancellation"
    );
    child.kill().await.expect("kill and reap child");
    rustix::fs::flock(
        &competing,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
    .expect("exclusion released only after child exit");
}
