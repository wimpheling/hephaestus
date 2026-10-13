use crate::fixture::{Fixture, finish_held, snapshot};
use recipe_application::InstallProgress;
use sqlx::PgPool;
use volume_trait::OwnedBackingPhase;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn exact_ready_reverifies_without_format_or_metadata_change_and_retains_custody(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    let (claim, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    assert_eq!(
        ready.observation.as_ref().unwrap().fields().phase,
        OwnedBackingPhase::Ready
    );
    let before = snapshot(&pool).await;
    let mut row_holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(
            request
                .purpose
                .receipt()
                .intent
                .registration()
                .id()
                .as_uuid(),
        )
        .fetch_one(&mut *row_holder)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        volume_trait::VolumeOwnedProvisioningRepository::assert_owned_first_ready_verification(
            f.worker.as_ref(),
            &ready.claim,
        ),
    )
    .await
    .expect("new readonly fence must not depend on a Volume row lock")
    .unwrap();
    row_holder.rollback().await.unwrap();
    assert_eq!(snapshot(&pool).await, before);
    let bytes = std::fs::read(request.purpose.canonical_path()).unwrap();
    let mut custody = f.local.verify_owned_ready(&ready.claim).await.unwrap();
    assert_eq!(custody.context(), &ready);
    assert_eq!(f.count("formats"), 1);
    assert_eq!(f.count("fscks"), 2);
    custody.revalidate().unwrap();
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        std::fs::read(request.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "same-volume verifier cannot enter during actual custody"
    );
    let progress = finish_held(&f, claim, custody).await;
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    let before = snapshot(&pool).await;
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "stale completed calls must not fsck an admitted runtime volume"
    );
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(f.count("fscks"), 2);
    assert_eq!(f.count("formats"), 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn dirty_or_substituted_ready_bytes_are_held_without_fsck_format_or_ready_overwrite(
    pool: PgPool,
) {
    use std::os::unix::fs::FileExt;
    let f = Fixture::new(&pool).await;
    let (_, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(request.purpose.canonical_path())
        .unwrap();
    file.write_all_at(&[0, 0], 1024 + 58).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let before = snapshot(&pool).await;
    let mut row_holder = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instance_state_volumes WHERE id=$1 FOR UPDATE")
        .bind(
            request
                .purpose
                .receipt()
                .intent
                .registration()
                .id()
                .as_uuid(),
        )
        .fetch_one(&mut *row_holder)
        .await
        .unwrap();
    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        volume_trait::VolumeOwnedProvisioningRepository::assert_owned_first_ready_verification(
            f.worker.as_ref(),
            &ready.claim,
        ),
    )
    .await
    .expect("new readonly fence must not depend on a Volume row lock")
    .unwrap();
    row_holder.rollback().await.unwrap();
    assert_eq!(snapshot(&pool).await, before);
    let bytes = std::fs::read(request.purpose.canonical_path()).unwrap();
    assert!(f.local.verify_owned_ready(&ready.claim).await.is_err());
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        std::fs::read(request.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 1));
    let path = request.purpose.canonical_path();
    let saved = path.with_extension("original");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::copy(&saved, &path).unwrap();
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "same bytes on a different inode cannot be adopted"
    );
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn foreign_actual_root_owner_cannot_reverify_the_original_ready_birth(pool: PgPool) {
    use std::{sync::Arc, time::Duration};
    use volume_local::{LocalVolumeConfig, LocalVolumeStore, OwnedVolumeMetadata, VolumeRootOwner};
    let f = Fixture::new(&pool).await;
    let (_, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let foreign = tempfile::TempDir::new().unwrap();
    let owner = Arc::new(VolumeRootOwner::initialize(foreign.path(), f.owner.host()).unwrap());
    let store = LocalVolumeStore::new(
        f.worker.clone(),
        LocalVolumeConfig {
            volume_root: foreign.path().into(),
            transient_runtime_roots: Vec::new(),
            host_id: f.owner.host().into(),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: f.tools.path().join("mkfs.ext4"),
        },
    )
    .unwrap()
    .with_owned_metadata(OwnedVolumeMetadata::new(
        f.actor.clone(),
        f.worker.clone(),
        owner,
    ))
    .unwrap();
    let before = snapshot(&pool).await;
    let entries = std::fs::read_dir(foreign.path()).unwrap().count();
    assert!(store.verify_owned_ready(&ready.claim).await.is_err());
    assert_eq!(std::fs::read_dir(foreign.path()).unwrap().count(), entries);
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 1));
}
