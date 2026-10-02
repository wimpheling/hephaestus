//! Real ext4 inspection at the exact-run attachment boundary; no VM claim.

use std::{fs, os::unix::fs::FileExt};

use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, ReleaseId, RunId};
use tempfile::TempDir;
use uuid::Uuid;
use volume_domain::{RunVolumeIdentity, RunVolumeSelections, VolumeAccessMode};
use volume_trait::{
    RunVolumeStore, VolumeError, VolumeMetadataRepository, VolumeProvisioningState,
};

use super::fixture;

#[tokio::test]
async fn known_ready_dirty_rw_keeps_bytes_while_ro_and_uncertain_backing_are_refused() {
    let root = TempDir::new().expect("private root");
    let mkfs = ["/usr/sbin/mkfs.ext4", "/sbin/mkfs.ext4"]
        .into_iter()
        .find(|path| std::path::Path::new(path).is_file())
        .expect("install e2fsprogs for native attachment tests");
    let (store, metadata, id) = fixture(&root, false, mkfs.into());
    let volume = store.provision(id).await.expect("real ready ext4");
    store
        .verify_run_backing(&volume, VolumeAccessMode::ReadOnly)
        .expect("clean RO backing");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&volume.host_path)
        .expect("owned file");
    let mut flags = [0; 4];
    file.read_exact_at(&mut flags, 1024 + 96)
        .expect("read flags");
    let flags = u32::from_le_bytes(flags) | 4;
    file.write_all_at(&0_u16.to_le_bytes(), 1024 + 58)
        .expect("dirty state");
    file.write_all_at(&flags.to_le_bytes(), 1024 + 96)
        .expect("journal recovery needed");
    file.sync_all().expect("dirty flags durable");
    drop(file);
    let before = fs::read(&volume.host_path).expect("dirty bytes");
    store
        .verify_run_backing(&volume, VolumeAccessMode::ReadWrite)
        .expect("known ready RW allows normal journal replay");
    assert!(matches!(
        store.verify_run_backing(&volume, VolumeAccessMode::ReadOnly),
        Err(VolumeError::ProvisioningUncertain(
            "read-only attachment requires a clean ext4 filesystem"
        ))
    ));
    let mut uncertain = volume.clone();
    uncertain.provisioning_state = VolumeProvisioningState::Uncertain;
    assert!(
        store
            .verify_run_backing(&uncertain, VolumeAccessMode::ReadWrite)
            .is_err()
    );
    let mut foreign = volume.clone();
    foreign.filesystem_uuid = Uuid::new_v4();
    assert!(
        store
            .verify_run_backing(&foreign, VolumeAccessMode::ReadWrite)
            .is_err()
    );
    assert_eq!(fs::read(&volume.host_path).expect("retained bytes"), before);
    assert_eq!(
        metadata.volume(id).await.expect("metadata unchanged"),
        volume
    );
}

#[tokio::test]
async fn missing_exact_metadata_fails_before_legacy_fallback_or_backing_effects() {
    let root = TempDir::new().expect("private root");
    let (store, _, _) = fixture(&root, false, "/must/not/execute/mkfs.ext4".into());
    let identity = RunVolumeIdentity::new(
        RunId::new(),
        AgentInstanceId::new(),
        AgentInstanceRevisionId::new(),
        ReleaseId::new(),
        ReleaseAgentId::new(),
        Uuid::new_v4(),
    )
    .expect("exact identity");
    let empty = RunVolumeSelections::new(identity, Vec::new()).expect("empty exact selection");
    assert!(matches!(
        store.acquire_run(&empty).await,
        Err(VolumeError::InvalidState(
            "exact-run volume metadata is not configured"
        ))
    ));
    assert!(store.load_run_selections(identity.run_id()).await.is_err());
    assert!(store.leases_for_run(identity.run_id()).await.is_err());
    assert_eq!(fs::read_dir(root.path()).expect("private root").count(), 0);
}
