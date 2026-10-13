use crate::{
    fixture::{Fixture, snapshot},
    fixtures,
};
use hephaestus_app::recipe_owned_volume::{OwnedVolumeCreateError, OwnedVolumeCreateFailure};
use recipe_application::{DeploymentError, InstallProgress};
use sqlx::PgPool;
use volume_trait::OwnedBackingPhase;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn ready_before_ledger_commit_recovers_original_and_never_formats_again(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    assert!(matches!(
        f.bad_recorder()
            .create_prepared_volume(
                &f.h.actor,
                f.h.command,
                f.h.intent.id(),
                &fixtures::key("data")
            )
            .await,
        Err(OwnedVolumeCreateError::Held {
            failure: OwnedVolumeCreateFailure::Completion(DeploymentError::AuthorizationDenied),
            ambiguity_error: None
        })
    ));
    let original = f.stored_context().await;
    assert_eq!(
        original.observation.as_ref().unwrap().fields().phase,
        OwnedBackingPhase::Ready
    );
    assert_eq!(f.count("formats"), 1);
    let before = std::fs::read(original.claim.purpose.canonical_path()).unwrap();
    let progress = f.create().await.unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    assert_eq!(f.stored_context().await, original);
    assert_eq!(
        std::fs::read(original.claim.purpose.canonical_path()).unwrap(),
        before
    );
    assert_eq!(f.count("formats"), 1);
    assert_eq!(f.count("fscks"), 3);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn ready_withdrawn_source_and_admitted_remove_stay_held_without_partial_or_not_applied(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    assert!(
        f.bad_recorder()
            .create_prepared_volume(
                &f.h.actor,
                f.h.command,
                f.h.intent.id(),
                &fixtures::key("data")
            )
            .await
            .is_err()
    );
    let current = f.stored_context().await;
    let bytes = std::fs::read(current.claim.purpose.canonical_path()).unwrap();
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(f.h.fixture.release)
        .execute(&pool)
        .await
        .unwrap();
    let before = snapshot(&pool).await;
    assert!(matches!(
        f.create().await,
        Err(OwnedVolumeCreateError::Deployment(
            DeploymentError::AuthorizationDenied
        ))
    ));
    assert_eq!(snapshot(&pool).await, before);
    let inspected =
        f.h.repository
            .inspect(&f.h.actor, f.h.intent.id())
            .await
            .unwrap();
    f.h.repository
        .admit_remove(
            &f.h.actor,
            fixtures::remove(&f.h.actor, f.h.intent.id(), inspected.version),
        )
        .await
        .unwrap();
    assert!(f.create().await.is_err());
    let rows:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_verifications WHERE outcome='not_applied'),(SELECT count(*) FROM volume_owned_partial_receipts),(SELECT count(*) FROM volume_owned_birth_retirements)").fetch_one(&pool).await.unwrap();
    assert_eq!(
        rows,
        (0, 0, 0),
        "Ready is neither partial custody nor absence; manager Ready retention remains separate"
    );
    assert_eq!(f.count("formats"), 1);
    assert_eq!(f.count("fscks"), 2);
    assert_eq!(
        std::fs::read(current.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn dirty_ready_replay_is_held_with_no_ready_overwrite_or_second_format(pool: PgPool) {
    use std::os::unix::fs::FileExt;
    let f = Fixture::new(&pool).await;
    assert!(
        f.bad_recorder()
            .create_prepared_volume(
                &f.h.actor,
                f.h.command,
                f.h.intent.id(),
                &fixtures::key("data")
            )
            .await
            .is_err()
    );
    let current = f.stored_context().await;
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(current.claim.purpose.canonical_path())
        .unwrap();
    file.write_all_at(&[0, 0], 1024 + 58).unwrap();
    file.sync_all().unwrap();
    drop(file);
    let before = std::fs::read(current.claim.purpose.canonical_path()).unwrap();
    let prior = current.clone();
    assert!(matches!(
        f.create().await,
        Err(OwnedVolumeCreateError::Held {
            failure: OwnedVolumeCreateFailure::Volume(_),
            ..
        })
    ));
    assert_eq!(f.stored_context().await, prior);
    assert_eq!(f.count("formats"), 1);
    assert_eq!(f.count("fscks"), 2, "bad superblock rejects before fsck");
    assert_eq!(
        std::fs::read(current.claim.purpose.canonical_path()).unwrap(),
        before
    );
}
