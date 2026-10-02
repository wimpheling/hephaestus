use heph_runtime::{VolumeError, VolumeMetadataRepository, VolumeState, VolumeStore};
use runtime_types::{RunId, VolumeId};
use volume_local::LocalVolumeStore;
use volume_postgres::PostgresVolumeMetadataRepository;

pub async fn assert_attached_lease_unchanged(
    store: &LocalVolumeStore,
    metadata: &PostgresVolumeMetadataRepository,
    volume: VolumeId,
    run: RunId,
    lease: &heph_runtime::VolumeLease,
) {
    let before = metadata.volume(volume).await.expect("attached metadata");
    assert_eq!(before.state, VolumeState::Attached);
    assert!(matches!(
        metadata.mark_ready(volume).await,
        Err(VolumeError::StaleLease)
    ));
    assert_eq!(
        metadata.volume(volume).await.expect("unchanged lifecycle"),
        before
    );
    assert_eq!(
        store
            .active_lease_for_run(run)
            .await
            .expect("unchanged active lease"),
        Some(lease.clone())
    );
}
