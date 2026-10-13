use crate::{fixture::Fixture, fixtures, volume_fixture::wait_insert};
use recipe_application::InstallProgress;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn engine_cancellation_preserves_detached_disk_custody_and_original_resume(pool: PgPool) {
    let f = Arc::new(Fixture::new(&pool).await);
    let mut blocker = pool.begin().await.unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *blocker)
        .await
        .unwrap();
    sqlx::query("LOCK TABLE recipe_effect_verifications IN SHARE MODE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let caller = {
        let f = f.clone();
        tokio::spawn(async move { f.resume().await })
    };
    wait_insert(&pool, pid, "INSERT INTO recipe_effect_verifications %").await;
    let original = f.stored_context().await;
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        f.local.verify_owned_ready(&original.claim).await.is_err(),
        "production creator owns the actual journal lock through its unresolved commit"
    );
    blocker.commit().await.unwrap();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let inspected = f.engine.inspect(&f.h.actor, f.h.intent.id()).await.unwrap();
            let progress = &inspected.ledger.resources[&fixtures::key("data")];
            if progress.install == InstallProgress::Ready && progress.active_attempt.is_none() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let counts = (f.count("formats"), f.count("fscks"));
    let complete = f.resume().await.unwrap();
    assert!(complete.activations[0].result.is_ok());
    assert_eq!((f.count("formats"), f.count("fscks")), counts);
    assert_eq!(f.stored_context().await, original);
}
