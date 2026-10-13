//! Real producer-bound142 secret checks; no mount/session/receipt rows are fabricated.
use super::fixture::Fixture;
use runtime_types::RunId;
use serde_json::Value;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const QUERY: &str = "SELECT verify_invocation142_empty_secret_cleanup($1,$2,$3,$4)";

pub async fn queued_denied(fixture: &Fixture, run: RunId) {
    // Empty history without owning processed Cancel and target is insufficient.
    let mut tx = transaction(&fixture.original.worker, true, true).await;
    let error = query(
        &mut tx,
        fixture,
        run.as_uuid(),
        &configuration(fixture),
        None,
    )
    .await
    .unwrap_err();
    denied(&error, &["P1180", "P1400"]);
    tx.rollback().await.unwrap();
}

pub async fn completed(fixture: &Fixture, run: RunId) {
    // Genuine controller cleanup and nonempty preservation precede this call.
    let config = configuration(fixture);
    let mut tx = transaction(&fixture.original.worker, true, true).await;
    let result = query(&mut tx, fixture, run.as_uuid(), &config, None)
        .await
        .unwrap();
    assert_eq!(
        result,
        serde_json::json!({"decision":"empty","run_id":run.as_uuid()})
    );
    tx.rollback().await.unwrap();
    let mut changed = config.clone();
    changed.push(b' '); // Equivalent JSON does not replace canonical bytes.
    context_denied(fixture, run.as_uuid(), &changed, None, &["P1400"]).await;
    context_denied(
        fixture,
        run.as_uuid(),
        &config,
        Some("foreign-secret-host"),
        &["P1400"],
    )
    .await;
    context_denied(fixture, Uuid::nil(), &config, None, &["23000"]).await;
    context_denied(fixture, Uuid::new_v4(), &config, None, &["P1180"]).await;
    role_and_transaction_denials(fixture, run, &config).await;
}

async fn role_and_transaction_denials(fixture: &Fixture, run: RunId, config: &[u8]) {
    for pool in [&fixture.admin, &fixture.original.application] {
        let mut tx = transaction(pool, true, true).await;
        let error = query(&mut tx, fixture, run.as_uuid(), config, None)
            .await
            .unwrap_err();
        denied(&error, &["42501"]);
        tx.rollback().await.unwrap();
    }
    for (repeatable, readonly) in [(false, true), (true, false)] {
        let mut tx = transaction(&fixture.original.worker, repeatable, readonly).await;
        let error = query(&mut tx, fixture, run.as_uuid(), config, None)
            .await
            .unwrap_err();
        denied(&error, &["23000"]);
        tx.rollback().await.unwrap();
    }
    // The facade may use this complete private137 helper; the worker cannot.
    let mut tx = transaction(&fixture.original.worker, true, true).await;
    let error = sqlx::query("SELECT invocation119_cleanup_secret_history_empty($1,$2,$3)")
        .bind(run.as_uuid())
        .bind(Uuid::nil())
        .bind(Uuid::nil())
        .execute(&mut *tx)
        .await
        .unwrap_err();
    denied(&error, &["42501"]);
    tx.rollback().await.unwrap();
}

async fn context_denied(
    fixture: &Fixture,
    run: Uuid,
    config: &[u8],
    host: Option<&str>,
    codes: &[&str],
) {
    let mut tx = transaction(&fixture.original.worker, true, true).await;
    let error = query(&mut tx, fixture, run, config, host)
        .await
        .unwrap_err();
    denied(&error, codes);
    tx.rollback().await.unwrap();
}

async fn transaction(pool: &PgPool, repeatable: bool, readonly: bool) -> Transaction<'_, Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query(if repeatable {
        "SET TRANSACTION ISOLATION LEVEL REPEATABLE READ"
    } else {
        "SET TRANSACTION ISOLATION LEVEL READ COMMITTED"
    })
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(if readonly {
        "SET TRANSACTION READ ONLY"
    } else {
        "SET TRANSACTION READ WRITE"
    })
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("SET LOCAL search_path = pg_catalog,public,pg_temp")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

async fn query(
    tx: &mut Transaction<'_, Postgres>,
    fixture: &Fixture,
    run: Uuid,
    config: &[u8],
    host: Option<&str>,
) -> Result<Value, sqlx::Error> {
    let provider = fixture.config.provider_scope();
    sqlx::query_scalar(QUERY)
        .bind(run)
        .bind(config)
        .bind(provider.namespace())
        .bind(host.unwrap_or_else(|| provider.host_id()))
        .fetch_one(&mut **tx)
        .await
}

fn configuration(fixture: &Fixture) -> Vec<u8> {
    serde_json::to_vec(&fixture.config.canonical_json()).unwrap()
}

fn denied(error: &sqlx::Error, codes: &[&str]) {
    let code = error.as_database_error().unwrap().code().unwrap();
    assert!(codes.contains(&code.as_ref()), "unexpected SQLSTATE {code}");
}
