//! Atomic worker ownership with real roles and private ordered 0108 DDL.

#[path = "cleanup/fixture.rs"]
// Shared upgrade and role helpers also contain unrelated cleanup scenarios.
#[allow(dead_code)]
mod fixture;
#[path = "planned_creation/normal.rs"]
mod normal;
#[path = "planned_creation/races.rs"]
mod races;
#[path = "planned_creation/seed.rs"]
mod seed;
#[path = "postgres/support.rs"]
// Shared exact provenance seed serves several independent adapter tests.
#[allow(dead_code)]
mod support;
#[path = "planned_creation/update.rs"]
mod update;

use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and private ordered 0108 DDL"]
async fn atomic_planned_runs_and_fresh_update_io_admissions(pool: PgPool) {
    fixture::checkpoint(107).run(&pool).await.unwrap();
    let historical = fixture::Fixture::seed(&pool).await;
    let before = historical.snapshot(&pool).await;
    let checksums = fixture::checksums(&pool).await;
    fixture::apply_private_checkpoint(&pool).await;
    assert_eq!(historical.snapshot(&pool).await, before);
    assert_eq!(fixture::checksums(&pool).await, checksums);
    normal::exercise(&pool, &historical.command).await;
    races::close_serializes_before_new_creation(&pool).await;
    update::exercise(&pool).await;
    println!(
        "REAL_ATOMIC_RUN_PLAN=1 upgrade107_preserved=1 immutable_inputs=1 exact_pins=1 worker_scope=1 historical_not_adopted=1 fresh_update_admission=1 closed_update_cleanup=1 crash_empty_target=1"
    );
}
