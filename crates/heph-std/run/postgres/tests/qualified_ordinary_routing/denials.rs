use run_orchestrator::RunRepository;
use run_postgres::QualifiedOrdinaryRoutingDecision;
use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::Fixture;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn positive_original_rewrites_are_rejected_by_actual_database_guards(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let before = f.original().await;
    let profile = sqlx::query("UPDATE runs SET execution_profile='owned_canonical' WHERE id=$1")
        .bind(f.start.run_id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap_err();
    integrity(&profile);
    let placement =
        sqlx::query("UPDATE run_legacy_vm_placements SET provider_namespace=$2 WHERE run_id=$1")
            .bind(f.start.run_id.as_uuid())
            .bind(Uuid::new_v4())
            .execute(&f.worker)
            .await
            .unwrap_err();
    integrity(&placement);
    let request = sqlx::query("UPDATE run_requests SET commit_sha=$2 WHERE run_id=$1")
        .bind(f.start.run_id.as_uuid())
        .bind("b".repeat(40))
        .execute(&f.worker)
        .await
        .unwrap_err();
    integrity(&request);
    let inbox = sqlx::query("UPDATE command_inbox SET payload=jsonb_set(payload,'{requires_state}','true'::jsonb) WHERE command_id=$1")
        .bind(f.start.command_id.as_uuid()).execute(&f.worker).await.unwrap_err();
    integrity(&inbox);
    assert_eq!(f.original().await, before);
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn contradictory_birth_rolls_back_without_inventing_an_original(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let mut changed = f.start.clone();
    changed.requires_state = true;
    assert!(
        f.runs
            .create_run_with_legacy_placement(&changed, &f.scope)
            .await
            .is_err()
    );
    let counts: (i64, i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM runs WHERE id=$1),
         (SELECT count(*) FROM run_legacy_vm_placements WHERE run_id=$1),
         (SELECT count(*) FROM command_inbox WHERE command_id=$2)",
    )
    .bind(f.start.run_id.as_uuid())
    .bind(f.start.command_id.as_uuid())
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(counts, (0, 0, 0));
    assert!(matches!(
        f.reader.inspect_ordinary_start(&f.start).await.unwrap(),
        QualifiedOrdinaryRoutingDecision::ProspectivelyUnborn(_)
    ));
    f.birth().await;
    f.original().await;
    f.close().await;
}

fn integrity(error: &sqlx::Error) {
    let error = error
        .as_database_error()
        .expect("actual PostgreSQL guard denial");
    assert_eq!(error.code().as_deref(), Some("23000"));
}
