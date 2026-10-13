use super::fixture::Fixture;
use run_domain::RunState;
use run_orchestrator::RunRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_domain::RunVolumeSelections;
use volume_trait::{RunVolumeLease, RunVolumeMetadataRepository};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL and native ext4 tools"]
async fn zero_complete_set_and_exact_replay(pool: PgPool) {
    complete(&pool, 0).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL and native ext4 tools"]
async fn two_real_backings_acquire_atomically_and_replay_same_fences(pool: PgPool) {
    complete(&pool, 2).await;
}
#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL and native ext4 tools"]
async fn thirty_two_real_backings_acquire_atomically_and_replay_same_fences(pool: PgPool) {
    complete(&pool, 32).await;
}
async fn complete(pool: &PgPool, count: usize) {
    let h = Fixture::new(pool, count).await;
    let created = h.consume().await.unwrap();
    assert!(!created.created, "115 precreated the exact Run");
    assert_eq!(created.run.id, h.start.run_id);
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selections = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    assert_eq!(selections.selections().len(), count);
    let now = OffsetDateTime::now_utc();
    let first = h
        .volumes()
        .acquire_run(
            &selections,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(first.len(), count);
    assert_acquisition_replay(&h, &selections, now, &first).await;
    h.repository()
        .bind_resources(h.start.run_id, None, None, None, &h.vm.0)
        .await
        .unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    let stable = h.rows().await;
    assert!(!h.consume().await.unwrap().created);
    assert_eq!(h.rows().await, stable);
    assert_execution_unsupported(&h, &stable).await;
    h.physical.assert_unchanged();
    h.birth.assert_historical_migrations(pool).await;
}

async fn assert_acquisition_replay(
    h: &Fixture,
    selections: &RunVolumeSelections,
    now: OffsetDateTime,
    expected: &[RunVolumeLease],
) {
    let before = h.rows().await;
    let replay = h
        .volumes()
        .acquire_run(
            selections,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(replay.as_slice(), expected);
    assert_eq!(
        h.rows().await,
        before,
        "replay may not reset generation or replace lease evidence"
    );
    assert!(!h.consume().await.unwrap().created);
    assert_eq!(h.rows().await, before);
}

async fn assert_execution_unsupported(h: &Fixture, stable: &serde_json::Value) {
    for state in [RunState::Starting, RunState::Running] {
        assert!(
            h.repository()
                .transition(h.start.run_id, state, None, None)
                .await
                .is_err(),
            "session authority is not enabled by119"
        );
        assert_eq!(&h.rows().await, stable);
    }
}
