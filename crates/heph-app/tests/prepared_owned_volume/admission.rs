use crate::fixture::{Fixture, snapshot, wait_insert};
use hephaestus_app::recipe_owned_volume::{OwnedVolumeCreateError, OwnedVolumeCreateFailure};
use recipe_application::DeploymentError;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};
use volume_trait::VolumeError;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn changed_whole_configuration_denies_before_claim_registration_or_host_io(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.h.prepared().await;
    let before = snapshot(&pool).await;
    let entries = std::fs::read_dir(f.root.path()).unwrap().count();
    assert!(matches!(
        f.drifted()
            .create_prepared_volume(
                &f.h.actor,
                f.h.command,
                f.h.intent.id(),
                &crate::fixtures::key("data")
            )
            .await,
        Err(OwnedVolumeCreateError::Deployment(_))
    ));
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(std::fs::read_dir(f.root.path()).unwrap().count(), entries);
    assert_eq!((f.count("formats"), f.count("fscks")), (0, 0));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn source_withdrawal_during_recorded_write_denies_first_format_and_preserves_primary_error(
    pool: PgPool,
) {
    let f = Arc::new(Fixture::new(&pool).await);
    let mut blocker = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    // Observe a real allocation/recording boundary; no arbitrary elapsed ordering.
    sqlx::query("LOCK TABLE volume_owned_backing_observations IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let caller = {
        let f = f.clone();
        tokio::spawn(async move { f.create().await })
    };
    wait_insert(
        &pool,
        pid,
        "INSERT INTO volume_owned_backing_observations(%",
    )
    .await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(f.h.fixture.release)
        .execute(&pool)
        .await
        .unwrap();
    blocker.commit().await.unwrap();
    let result = tokio::time::timeout(Duration::from_secs(10), caller)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(
            result,
            Err(OwnedVolumeCreateError::Held {
                failure: OwnedVolumeCreateFailure::Volume(VolumeError::PermissionDenied),
                ambiguity_error: Some(DeploymentError::AuthorizationDenied)
            })
        ),
        "denied ambiguity recording must not replace the fresh actor's primary denial"
    );
    assert_eq!((f.count("formats"), f.count("fscks")), (0, 0));
    let history:(i64,i64,i64) = sqlx::query_as("SELECT (SELECT count(*) FROM volume_owned_provisioning_operations),(SELECT count(*) FROM recipe_effect_verifications),(SELECT count(*) FROM volume_owned_backing_observations WHERE phase='format_intent')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(history, (1, 0, 0));
    let current = f.stored_context().await;
    assert_eq!(current.claim.generation, 1);
    assert_eq!(
        current.observation.as_ref().unwrap().fields().phase,
        volume_trait::OwnedBackingPhase::Recorded
    );
    let bytes = std::fs::read(current.claim.purpose.canonical_path()).unwrap();
    assert!(
        bytes.iter().all(|byte| *byte == 0),
        "allocated raw birth has no filesystem"
    );
    let before = snapshot(&pool).await;
    assert!(f.create().await.is_err());
    assert_eq!(snapshot(&pool).await, before);
    assert_eq!(
        std::fs::read(current.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
}
