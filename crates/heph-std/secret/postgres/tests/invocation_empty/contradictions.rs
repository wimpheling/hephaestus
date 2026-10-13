use super::{fixture::Fixture, secret_rows::SecretRows};
use sqlx::PgPool;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn materialized_and_destroyed_mount_records_both_contradict_empty_family(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    f.allowed_unchanged().await;
    sqlx::query("INSERT INTO secret_runtime_mounts(run_id,opaque_directory,state) VALUES($1,$2,'materialized')")
        .bind(f.authority.birth.start.run_id.as_uuid()).bind(Uuid::new_v4())
        .execute(&pool).await.unwrap();
    f.denied_unchanged().await;
    sqlx::query(
        "UPDATE secret_runtime_mounts SET state='destroyed',destroyed_at=now() WHERE run_id=$1",
    )
    .bind(f.authority.birth.start.run_id.as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    f.denied_unchanged().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn unreferenced_revoked_selected_revision_binding_and_rule_are_contradictions(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let rows = SecretRows::new(&f).await;
    f.allowed_unchanged().await;
    rows.binding(&f).await;
    let eligible: bool = sqlx::query_scalar("SELECT invocation119_authority_eligible($1,$2,false)")
        .bind(f.authority.birth.start.run_id.as_uuid())
        .bind(serde_json::to_vec(&f.authority.birth.config.canonical_json()).unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(
        eligible,
        "the additional empty boundary detects actual unreferenced secret rows"
    );
    f.denied_unchanged().await;
    rows.rule(&f).await;
    f.denied_unchanged().await;
    let saved: serde_json::Value =
        sqlx::query_scalar("SELECT secret_bindings FROM agent_instance_revisions WHERE id=$1")
            .bind(f.authority.birth.start.instance_revision_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        saved,
        serde_json::json!([]),
        "an empty label cannot hide actual binding rows"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn historical_secret_provenance_revoked_session_and_released_lease_graph_denies(
    pool: PgPool,
) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    f.authority.issue().await;
    // Actual115 rejects historical run_instance_provenance as conflicting
    // lineage. Retain the exact previously loaded Run to exercise the verifier
    // after this legal raw contradiction; never fabricate a replacement Run.
    let run = f.run().await;
    SecretRows::instance_provenance(&f).await;
    f.denied_run_unchanged(&run).await;
    let rows = SecretRows::new(&f).await;
    rows.binding(&f).await;
    rows.rule(&f).await;
    rows.provenance(&f).await;
    rows.session(&f).await;
    rows.lease_and_snapshot(&f).await;
    assert_eq!(
        super::secret_rows::count(&pool, f.authority.birth.start.run_id.as_uuid()).await,
        (1, 1, 1, 1),
        "actual schema-valid linked rows, not a fake absent query"
    );
    f.denied_run_unchanged(&run).await;
    // FK dependencies require binding/provenance rows for the child graph;
    // this is whole-graph contradiction coverage, not isolated child predicates.
}
