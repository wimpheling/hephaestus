//! Negative fault transactions roll back; no fabricated positive provenance.

use run_domain::CancelRun;
use sqlx::{PgPool, Postgres, Transaction};

use super::{fixture::Fixture, snapshot};

pub(super) async fn pending_inbox_and_changed_origin(
    pool: &PgPool,
    f: &Fixture,
    cancel: &CancelRun,
) {
    let original = snapshot::state(pool, f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE command_inbox SET processed_at=NULL WHERE command_id=$1")
        .bind(cancel.command_id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = discover(&mut tx, f).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P1400")
    );
    tx.rollback().await.unwrap();
    assert_eq!(original, snapshot::state(pool, f).await);

    let mut tx = pool.begin().await.unwrap();
    sqlx::query("UPDATE outbox SET payload=jsonb_set(payload,'{reason}',to_jsonb('contradictory origin'::text)) WHERE aggregate_id=$1 AND event_type='run.cancel_requested'")
        .bind(f.start.run_id.as_uuid()).execute(&mut *tx).await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = discover(&mut tx, f).await.unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("P1400")
    );
    tx.rollback().await.unwrap();
    assert_eq!(original, snapshot::state(pool, f).await);
}

async fn discover(tx: &mut Transaction<'_, Postgres>, f: &Fixture) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT * FROM discover_ordinary_queued_cancel_context140($1,$2,$3,$4)")
        .bind(f.start.run_id.as_uuid())
        .bind(f.host.provider_namespace())
        .bind(f.host.host_id())
        .bind(serde_json::to_vec(&f.config.canonical_json()).unwrap())
        .execute(&mut **tx)
        .await
        .map(|_| ())
}

pub(super) async fn held_original_reservation(f: &Fixture, cancel: &CancelRun) {
    let mut holder = f.original.worker.begin().await.unwrap();
    let key: i64 = sqlx::query_scalar("SELECT command_mutex FROM invocation119_reservation_context($1) WHERE lock_class=0 ORDER BY command_mutex LIMIT 1")
        .bind(f.start.run_id.as_uuid()).fetch_one(&mut *holder).await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(key)
        .execute(&mut *holder)
        .await
        .unwrap();
    let mut contender = f.original.worker.begin().await.unwrap();
    let error = sqlx::query("SELECT lock_ordinary_cancel_delivery140($1,$2,$3,$4,$5)")
        .bind(f.start.run_id.as_uuid())
        .bind(serde_json::to_value(cancel).unwrap())
        .bind(f.host.provider_namespace())
        .bind(f.host.host_id())
        .bind(serde_json::to_vec(&f.config.canonical_json()).unwrap())
        .execute(&mut *contender)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("55P03")
    );
    contender.rollback().await.unwrap();
    holder.rollback().await.unwrap();
}

// Role/isolation failures must precede observation, and an exact RRRO unknown
// claim must fail closed instead of producing an empty successful inventory.
pub(super) async fn drain_observation_boundaries(pool: &PgPool, f: &Fixture) {
    for (role, isolation, code) in [
        (
            "SET LOCAL ROLE hephaestus_app",
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "42501",
        ),
        (
            "SET LOCAL ROLE hephaestus_worker",
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED READ WRITE",
            "P1400",
        ),
        (
            "SET LOCAL ROLE hephaestus_worker",
            "SET TRANSACTION ISOLATION LEVEL READ COMMITTED READ ONLY",
            "P1400",
        ),
        (
            "SET LOCAL ROLE hephaestus_worker",
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ WRITE",
            "P1400",
        ),
        (
            "SET LOCAL ROLE hephaestus_worker",
            "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY",
            "P1400",
        ),
    ] {
        drain_observation_denied(pool, f, role, isolation, code).await;
    }
    let original = snapshot::state(pool, f).await;
    let neighbors = snapshot::neighbors(pool, f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    let error = sqlx::query("SELECT invocation119_cancel_original($1,NULL)")
        .bind(f.start.run_id.as_uuid())
        .execute(&mut *tx)
        .await
        .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    assert_eq!(original, snapshot::state(pool, f).await);
    assert_eq!(neighbors, snapshot::neighbors(pool, f).await);
}

async fn drain_observation_denied(
    pool: &PgPool,
    f: &Fixture,
    role: &str,
    isolation: &str,
    expected_code: &str,
) {
    let original = snapshot::state(pool, f).await;
    let neighbors = snapshot::neighbors(pool, f).await;
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(isolation).execute(&mut *tx).await.unwrap();
    sqlx::query(role).execute(&mut *tx).await.unwrap();
    // Negative unknown inputs only; no positive origin/claim is seeded.
    let error =
        sqlx::query("SELECT * FROM observe_prepared_instance_drain140($1,$2,$3,$4,$5,$6,$7,$8,$9)")
            .bind(f.start.instance_id.as_uuid())
            .bind(uuid::Uuid::new_v4())
            .bind(serde_json::json!({}))
            .bind(uuid::Uuid::new_v4())
            .bind(serde_json::json!({}))
            .bind(serde_json::json!({}))
            .bind(f.host.provider_namespace())
            .bind(f.host.host_id())
            .bind(serde_json::to_vec(&f.config.canonical_json()).unwrap())
            .execute(&mut *tx)
            .await
            .unwrap_err();
    assert_eq!(
        error.as_database_error().unwrap().code().as_deref(),
        Some(expected_code)
    );
    tx.rollback().await.unwrap();
    assert_eq!(original, snapshot::state(pool, f).await);
    assert_eq!(neighbors, snapshot::neighbors(pool, f).await);
}
