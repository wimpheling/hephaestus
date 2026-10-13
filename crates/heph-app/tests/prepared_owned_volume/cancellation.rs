use crate::fixture::{Fixture, wait_insert};
use recipe_application::InstallProgress;
use sqlx::PgPool;
use std::{sync::Arc, time::Duration};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private113 PostgreSQL and native e2fsprogs"]
async fn production_completion_owner_survives_caller_cancellation_and_keeps_actual_custody(
    pool: PgPool,
) {
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
        tokio::spawn(async move { f.create().await })
    };
    wait_insert(&pool, pid, "INSERT INTO recipe_effect_verifications %").await;
    let original = f.stored_context().await;
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 2));
    caller.abort();
    assert!(caller.await.unwrap_err().is_cancelled());
    assert!(
        f.local.verify_owned_ready(&original.claim).await.is_err(),
        "production detached completion owns actual journal flock during the blocked recorder transaction"
    );
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
    assert_eq!(
        f.create().await.unwrap(),
        progress,
        "completed replay requires no discovery or physical check"
    );
    assert_eq!(f.stored_context().await, original);
    assert_eq!((f.count("formats"), f.count("fscks")), (1, 2));
    let completed: bool = sqlx::query_scalar("SELECT recipe_owned_first_birth_completed($1)")
        .bind(
            original
                .claim
                .purpose
                .receipt()
                .intent
                .registration()
                .id()
                .as_uuid(),
        )
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(completed);
}
