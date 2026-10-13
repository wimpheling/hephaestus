//! Canonical worker cleanup evidence; requires the ordered 0108 checkpoint.

#[path = "cleanup/fixture.rs"]
mod fixture;
#[path = "cleanup/ownership.rs"]
mod ownership;
#[path = "cleanup/scenarios.rs"]
mod scenarios;
#[path = "postgres/support.rs"]
// This shared provenance seed also serves mailbox and request tests.
#[allow(dead_code)]
mod support;

use fixture::{Fixture, checkpoint};
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and ordered migration 0108"]
async fn canonical_cleanup_upgrade_scope_fences_and_replay(pool: PgPool) {
    checkpoint(107).run(&pool).await.expect("historical schema");
    let historical = Fixture::seed(&pool).await;
    let before = historical.snapshot(&pool).await;
    let checksums = fixture::checksums(&pool).await;
    fixture::apply_private_checkpoint(&pool).await;
    let exists: bool = sqlx::query_scalar("SELECT to_regclass('run_cleanup_targets') IS NOT NULL")
        .fetch_one(&pool)
        .await
        .expect("cleanup checkpoint present");
    assert!(
        exists,
        "publish the coordinated 0108 checkpoint before running this proof"
    );
    assert_eq!(historical.snapshot(&pool).await, before);
    assert_eq!(fixture::checksums(&pool).await, checksums);
    let fixture = Fixture::seed_planned(&pool, "cleanup-host-1").await;
    let foreign_host = Fixture::seed_planned(&pool, "foreign-host").await;
    let incomplete = Fixture::seed_planned(&pool, "cleanup-host-1").await;
    scenarios::scope_and_immutable_target(&pool, &fixture).await;
    ownership::foreign_host_stays_held(&pool, &foreign_host).await;
    ownership::incomplete_snapshot_rolls_back(&pool, &incomplete).await;
    scenarios::fences_and_completion(&pool, &fixture).await;
    scenarios::empty_and_historical(&pool).await;
    scenarios::acquisition_close_race(&pool).await;
    println!(
        "REAL_RUN_CLEANUP=1 upgrade107_preserved=1 worker_scope=1 complete_set=1 atomic_rollback=1 raw_bypass_denied=1 receipt_replay=1 empty_set=1 historical_held=1 acquire_close_serialized=1"
    );
}
