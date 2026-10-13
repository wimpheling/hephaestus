use super::{
    controller::{Control, build},
    observations::{rows, unconsumed},
    queued_fixture::Queued,
};
use run_orchestrator::{RunCleanupWakeProgress, RunRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn original_cleanup_survives_actual_source_and_user_withdrawal(pool: PgPool) {
    let h = Queued::new(&pool, 2, true).await;
    let before = h.prepare().await;
    let volumes = rows(&pool).await["agent_instance_state_volumes"].clone();
    let closure = crate::prior_observations::preserved_closure(&pool).await;
    sqlx::query("UPDATE users SET status='disabled' WHERE id=$1")
        .bind(h.fixture.birth.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(h.fixture.birth.fixture.release)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(h.read().await, before);
    assert_eq!(h.prepare().await, before);
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap(),
        before.run
    );
    let core = build(&h, Control::new(), None, None);
    assert_eq!(
        core.wake_cleanup(&h.request).await.unwrap(),
        RunCleanupWakeProgress::Complete
    );
    assert_eq!(
        crate::prior_observations::preserved_closure(&pool).await,
        closure
    );
    let after = rows(&pool).await;
    let counts:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM run_cleanup_receipts WHERE run_id=$1),(SELECT count(*) FROM run_cleanup_completions WHERE run_id=$1)")
        .bind(h.request.run_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1));
    assert_eq!(after["agent_instance_state_volumes"], volumes);
    unconsumed(&h).await;
    h.fixture.physical.assert_unchanged();
    // Fresh-B pre117 resume remains a separately unmet admission/reader boundary.
}
