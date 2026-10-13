use super::{
    observations::{held, rows, unconsumed},
    queued_fixture::Queued,
};
use run_orchestrator::{RunCleanupRepository, RunRepository};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn genuine_candidate_configured_cancel_preparation_read_and_get(pool: PgPool) {
    let h = Queued::new(&pool, 2, false).await;
    super::admission::boundaries(&pool, &h).await;
    super::cancel::before_delivery(&h).await;
    h.deliver().await;
    let before = rows(&pool).await;
    held(&h.repository().get(h.request.run_id()).await.unwrap_err());
    held(
        &h.repository()
            .load_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    held(
        &h.cleanup()
            .begin_cleanup(h.request.run_id())
            .await
            .unwrap_err(),
    );
    held(
        &h.cleanup()
            .recorded_cleanup_receipt(h.request.run_id())
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, before);
    let prepared = h.prepare().await;
    assert_eq!(prepared.run.state, run_domain::RunState::Queued);
    assert_eq!(prepared.run.outcome, None);
    let target = h.target().await;
    assert!(target.leases().is_empty());
    assert_eq!(target.vm_target().vm_id(), Some(&h.fixture.vm));
    assert_eq!(h.read().await, prepared);
    assert_eq!(
        h.repository().get(h.request.run_id()).await.unwrap(),
        prepared.run
    );
    let after = rows(&pool).await;
    crate::prior_observations::preparation_events(&pool, &before, &after, h.request.run_id(), 0)
        .await;
    super::observations::preparation_delta(&h, &before, &after);
    assert_eq!(h.prepare().await, prepared);
    assert_eq!(rows(&pool).await, after);
    super::cancel::after_delivery(&h).await;
    unconsumed(&h).await;
    h.fixture.physical.assert_unchanged();
    h.fixture.birth.assert_historical_migrations(&pool).await;
}
