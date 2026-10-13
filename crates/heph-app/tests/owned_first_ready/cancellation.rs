use crate::fixture::{Fixture, finish_held};
use recipe_application::InstallProgress;
use sqlx::PgPool;
use std::time::Duration;
use volume_trait::VolumeOwnedProvisioningRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn caller_cancellation_cannot_release_custody_during_an_inflight_completion_commit(
    pool: PgPool,
) {
    let f = std::sync::Arc::new(Fixture::new(&pool).await);
    let (claim, request) = f.request().await;
    let ready = f.local.provision_owned(&f.h.actor, &request).await.unwrap();
    let custody = f.local.verify_owned_ready(&ready.claim).await.unwrap();
    let mut blocker = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    // SHARE allows all readonly fence/claim checks but blocks real worker INSERT.
    sqlx::query("LOCK TABLE recipe_effect_verifications IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let caller = {
        let f = f.clone();
        tokio::spawn(async move { finish_held(&f, claim, custody).await })
    };
    wait_worker_insert(&pool, pid).await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        f.local.verify_owned_ready(&ready.claim).await.is_err(),
        "detached completion still owns actual journal flock after caller cancellation"
    );
    assert_eq!(f.count("fscks"), 2);
    blocker.commit().await.unwrap();
    let progress = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot =
                f.h.repository
                    .inspect(&f.h.actor, f.h.intent.id())
                    .await
                    .unwrap();
            let progress = &snapshot.resources[&crate::fixtures::key("data")];
            if progress.install == InstallProgress::Ready && progress.active_attempt.is_none() {
                break progress.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(progress.install, InstallProgress::Ready);
    assert!(
        f.worker
            .assert_owned_first_ready_verification(&ready.claim)
            .await
            .is_err()
    );
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 2));
}
async fn wait_worker_insert(pool: &PgPool, blocker: i32) {
    tokio::time::timeout(Duration::from_secs(10),async {
        loop {
            let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity activity WHERE activity.datname=current_database() AND pg_blocking_pids(activity.pid) @> ARRAY[$1::integer] AND activity.query LIKE 'INSERT INTO recipe_effect_verifications %')")
                .bind(blocker).fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::task::yield_now().await;
        }
    }).await.expect("actual recorder reaches exact PostgreSQL lock barrier, not an elapsed scheduling assumption");
}
