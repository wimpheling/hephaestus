use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementConsumption, RunState};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use sqlx::PgPool;

use super::{
    fixture,
    public_fixture::{Role, role_pool},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn preexisting_profile_and_public_history_cannot_acquire_birth_eligibility(pool: PgPool) {
    // Genuine upgrade boundary: create qualified Legacy rows BEFORE the new
    // fragment, then prove its default false marker cannot be retrofitted.
    super::base::schema(&pool).await;
    let f = super::base::Fixture::new(&pool, true).await;
    let lease = f.acquire().await.unwrap();
    f.to_running(Some(&lease)).await;
    let before = super::base::evidence(&pool, f.command.run_id).await;
    let checksums = super::public_fixture::checksums(&pool).await;
    sqlx::raw_sql(include_str!("/var/tmp/0108_legacy_vm_placement.sql"))
        .execute(&pool)
        .await
        .unwrap();
    let owner = fixture::scope();
    let configured = f.runs.clone().with_legacy_vm_placement(owner.clone());
    assert!(
        configured
            .legacy_vm_placement(f.command.run_id, &owner)
            .await
            .is_err()
    );
    assert!(
        configured
            .create_run_with_legacy_placement(&f.command, &owner)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE runs SET legacy_vm_placement_required=true WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    let mut after = super::base::evidence(&pool, f.command.run_id).await;
    let marker = after["run"]
        .as_object_mut()
        .unwrap()
        .remove("legacy_vm_placement_required");
    assert_eq!(marker, Some(serde_json::json!(false)));
    assert_eq!(before, after);
    assert_eq!(checksums, super::public_fixture::checksums(&pool).await);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn nonworker_cannot_create_read_or_consume_placement(pool: PgPool) {
    fixture::schema(&pool).await;
    let privileges: (bool, bool, bool) = sqlx::query_as("SELECT has_function_privilege('hephaestus_worker','run_profile_producer_matches(runs)','EXECUTE'),has_function_privilege('hephaestus_app','run_profile_producer_matches(runs)','EXECUTE'),EXISTS(SELECT 1 FROM pg_proc function CROSS JOIN LATERAL aclexplode(COALESCE(function.proacl,acldefault('f',function.proowner))) privilege WHERE function.oid='run_profile_producer_matches(runs)'::regprocedure AND privilege.grantee=0 AND privilege.privilege_type='EXECUTE')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(privileges, (true, false, false));
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, false, &owner).await;
    f.runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    let app = role_pool(&pool, Role::Actor).await;
    let actor = PgRunRepository::new(app.clone())
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(owner.clone());
    assert!(
        actor
            .legacy_vm_placement(f.command.run_id, &owner)
            .await
            .is_err()
    );
    assert!(
        actor
            .consume_legacy_vm_placement(
                &f.command,
                &owner,
                LegacyVmPlacementConsumption::Provision
            )
            .await
            .is_err()
    );
    assert!(
        actor
            .create_run_with_legacy_placement(&f.command, &owner)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM run_legacy_vm_placements WHERE run_id=$1")
            .bind(f.command.run_id.as_uuid())
            .fetch_all(&app)
            .await
            .is_err()
    );
    app.close().await;
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private placement PostgreSQL"]
async fn birth_scope_and_consumption_cannot_be_rewritten_or_deleted(pool: PgPool) {
    fixture::schema(&pool).await;
    let owner = fixture::scope();
    let f = fixture::fresh(&pool, false, &owner).await;
    f.runs
        .create_run_with_legacy_placement(&f.command, &owner)
        .await
        .unwrap();
    assert!(
        sqlx::query("UPDATE run_legacy_vm_placements SET host_id='foreign-host' WHERE run_id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM run_legacy_vm_placements WHERE run_id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    assert!(
        f.runs
            .bind_resources(
                f.command.run_id,
                None,
                None,
                None,
                &runtime_types::RunId::new().to_string()
            )
            .await
            .is_err()
    );
    assert_eq!(
        f.runs.get(f.command.run_id).await.unwrap().state,
        RunState::Queued
    );
    f.worker.close().await;
}
