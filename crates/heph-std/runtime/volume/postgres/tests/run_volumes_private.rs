//! Disposable private-108 database semantics. No VM, dispatch, or host-I/O claim.
//! Remove the private SQL loader when the complete migration is published.

use sqlx::PgPool;

#[path = "private_migrations/mod.rs"]
mod private_migrations;

#[path = "../../../../authorization/authz-postgres/tests/postgres/seed.rs"]
mod seed;
// Reuse the canonical cross-context graph; unrelated matrix helpers are unused.
#[path = "run_volumes_private/fixture.rs"]
mod fixture;
#[path = "run_volumes_private/historical_update.rs"]
mod historical_update;
#[path = "run_volumes_private/legacy.rs"]
mod legacy;
#[path = "run_volumes_private/named.rs"]
mod named;
#[path = "run_volumes_private/scalar.rs"]
mod scalar;
#[allow(dead_code)]
#[path = "../../../../authorization/authz-postgres/tests/postgres/support.rs"]
mod support;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and the two explicitly private 108 SQL fragments"]
async fn exact_legacy_origin_upgrade_replay_and_candidate_hook(pool: PgPool) {
    let seeded = fixture::baseline(&pool).await;
    legacy::exercise(&pool, &seeded).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and the two explicitly private 108 SQL fragments"]
async fn named_full_sets_and_live_authority_are_atomic(pool: PgPool) {
    let seeded = fixture::baseline(&pool).await;
    named::exercise(&pool, &seeded).await;
}
