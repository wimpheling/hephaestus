use super::{
    fixture::{Fixture, code},
    secret_rows::SecretRows,
};
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn saved_empty_guards_and_reachable_runtime_secret_history_deny_honestly(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    f.allowed().await;
    for value in [
        serde_json::json!([Uuid::new_v4()]),
        serde_json::json!({"not":"array"}),
        serde_json::Value::Null,
    ] {
        let before = f.rows().await;
        let error =
            sqlx::query("UPDATE agent_instance_revisions SET secret_bindings=$1 WHERE id=$2")
                .bind(value)
                .bind(f.run.instance_revision_id.as_uuid())
                .execute(&pool)
                .await
                .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("23000"));
        assert_eq!(f.rows().await, before);
        // This proves immutable [] exclusion, not an admitted nonempty producer.
        f.allowed().await;
    }
    let before = f.rows().await;
    let mut tx = f.birth.birth.worker.begin().await.unwrap();
    let error = sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$1,$2,$3,$4,invocation119_snapshot_hash($1,$4))")
        .bind(f.run.id.as_uuid()).bind(f.run.instance_id.as_uuid()).bind(f.run.instance_revision_id.as_uuid())
        .bind(AUTHORIZATION_MODEL_VERSION).execute(&mut *tx).await.unwrap_err();
    // The B1 BEFORE INSERT guard excludes this ceiling at Leasing. No partial
    // snapshot becomes visible to the facade, even inside the transaction.
    assert_eq!(code(&error).as_deref(), Some("P1192"));
    tx.rollback().await.unwrap();
    assert_eq!(
        f.rows().await,
        before,
        "rejected partial snapshot and rollback preserve the whole metadata graph"
    );
    f.allowed().await;
    let rows = SecretRows::new(&f).await;
    // The actual Invocation session requires provenance which 115 excludes.
    // Assert that FK exclusion before adding any own contradiction metadata.
    assert_session_fk_exclusion(&f, &rows).await;
    // Own mount metadata is independently reachable with immutable/open/caps
    // still true. Materialized and destroyed states both count as history.
    sqlx::query("INSERT INTO secret_runtime_mounts(run_id,opaque_directory,state) VALUES($1,$2,'materialized')")
        .bind(f.run.id.as_uuid()).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    let eligible:(bool,bool,bool)=sqlx::query_as("SELECT invocation119_immutable($1),invocation119_open_set($1),invocation119_capabilities_live($1)")
        .bind(f.run.id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        eligible,
        (true, true, true),
        "own mount alone contradicts first acquisition"
    );
    f.denied(&f.run).await;
    assert!(
        !f.raw(
            &f.birth.birth.worker,
            &f.run,
            &f.config(),
            AUTHORIZATION_MODEL_VERSION
        )
        .await
        .unwrap()
    );
    sqlx::query(
        "UPDATE secret_runtime_mounts SET state='destroyed',destroyed_at=now() WHERE run_id=$1",
    )
    .bind(f.run.id.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    f.denied(&f.run).await;
    // Owner-pool adversarial metadata: schema update-phase provenance can refer
    // to this existing Invocation, but 115 rejects that contradiction. All later
    // linked secret rows are denial-only whole-graph coverage, not isolated
    // predicates or a valid session/dispatch/execution producer.
    SecretRows::instance_provenance(&f).await;
    let immutable: bool = sqlx::query_scalar("SELECT invocation119_immutable($1)")
        .bind(f.run.id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!immutable);
    f.denied(&f.run).await;
    rows.session(&f).await.unwrap();
    let status: String =
        sqlx::query_scalar("SELECT status FROM secret_runtime_sessions WHERE run_id=$1")
            .bind(f.run.id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(status, "revoked");
    f.denied(&f.run).await;
    rows.binding(&f).await;
    rows.rule(&f).await;
    rows.provenance(&f).await;
    rows.lease_and_snapshot(&f).await;
    assert_eq!(
        super::secret_rows::count(&pool, f.run.id.as_uuid()).await,
        (1, 1, 1, 1)
    );
    f.denied(&f.run).await;
    // Keep the actually returned Run as comparison data after lineage denial.
    assert!(
        !f.raw(
            &f.birth.birth.worker,
            &f.run,
            &f.config(),
            AUTHORIZATION_MODEL_VERSION
        )
        .await
        .unwrap()
    );
    f.birth.physical.assert_unchanged();
}

async fn assert_session_fk_exclusion(f: &Fixture, rows: &SecretRows) {
    let before = f.rows().await;
    let error = rows.session(f).await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("23503"));
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::constraint),
        Some("secret_runtime_sessions_run_id_fkey1")
    );
    assert_eq!(f.rows().await, before);
    f.allowed().await;
}
