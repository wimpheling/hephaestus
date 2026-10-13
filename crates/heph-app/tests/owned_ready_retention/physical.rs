use crate::fixture::Fixture;
use recipe_application::OwnedReadyRetentionRepository;
use sqlx::PgPool;
use std::os::unix::fs::FileExt;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn dirty_ready_is_held_without_format_metadata_rewrite_or_false_retention(pool: PgPool) {
    let f = Fixture::new(&pool, false).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(f.ready.claim.purpose.canonical_path())
        .unwrap();
    file.write_all_at(&[0, 0], 1024 + 58).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let bytes = std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap();
    let before = f.snapshot().await;
    assert!(
        f.coordinator()
            .retain(&identity, &request, &f.ready.claim)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(
        std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!((f.birth.count("formats"), f.birth.count("fscks")), (1, 1));
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn copied_backing_inode_cannot_inherit_ready_retention_custody(pool: PgPool) {
    let f = Fixture::new(&pool, false).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let path = f.ready.claim.purpose.canonical_path();
    let saved = f.birth.root.path().join("retained-original.raw");
    std::fs::rename(&path, &saved).unwrap();
    std::fs::copy(&saved, &path).unwrap();
    let before = f.snapshot().await;
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        f.coordinator()
            .retain(&identity, &request, &f.ready.claim)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read(saved).unwrap(), bytes);
    assert_eq!((f.birth.count("formats"), f.birth.count("fscks")), (1, 1));
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn unqualified_preexisting_consumer_is_held_without_guessing_terminal_history(pool: PgPool) {
    let f = Fixture::new(&pool, true).await;
    // Real unchanged107 typed import: selected binding/grant is legal, but this
    // fresh generic consumer has no original113 recipe/106 drain lineage.
    let prepared = f.birth.h.prepared().await;
    let mut command = hephaestus_app::recipe_instance_import::to_import_command(&prepared);
    command.instance_id = release_domain::AgentInstanceId::from_uuid(uuid::Uuid::new_v4());
    command.revision_id = release_domain::AgentInstanceRevisionId::from_uuid(uuid::Uuid::new_v4());
    command.command_key = release_domain::ReleaseCommandKey::derive(
        "unqualified_ready_consumer",
        &[command.instance_id.as_uuid().as_bytes()],
    );
    command.name = release_domain::InstanceName::parse("unqualified-ready-consumer").unwrap();
    command.volumes[0].grant_id =
        volume_domain::VolumeMountGrantId::from_uuid(uuid::Uuid::new_v4()).unwrap();
    let consumer = f
        .birth
        .h
        .release
        .import_agent_with_volumes(&f.birth.h.actor, command)
        .await
        .unwrap();
    let actual:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_id=$1 AND volume_id=$2),(SELECT count(*) FROM recipe_prepared_instance_imports WHERE instance_id=$1)")
        .bind(consumer.as_uuid()).bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid())
        .fetch_one(&pool).await.unwrap();
    assert_eq!(
        actual,
        (1, 0),
        "actual legal consumer binding; no invented prepared/drained lineage"
    );
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.actor
        .begin_owned_ready_retention(&identity, &request)
        .await
        .unwrap();
    let before = f.snapshot().await;
    let fscks = f.birth.count("fscks");
    assert!(
        f.coordinator()
            .retain(&identity, &request, &f.ready.claim)
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    assert_eq!(f.birth.count("fscks"), fscks);
    assert_eq!(f.birth.count("formats"), 1);
    drop(f);
}
