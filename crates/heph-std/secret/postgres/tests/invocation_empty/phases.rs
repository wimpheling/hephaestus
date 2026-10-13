use heph_secret::SecretMountMetadata;
use run_domain::RunState;
use run_orchestrator::{RunRepository, RunSecretManager};
use secret_postgres::PostgresSecretMountMetadata;
use sqlx::PgPool;
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn genuine_leasing_fullset_and_bound_vm_cannot_claim_provisioning_empty_permission(
    pool: PgPool,
) {
    let birth = Box::pin(crate::fixture::Fixture::new(&pool, 0)).await;
    birth.consume().await.unwrap();
    birth
        .repository()
        .transition(birth.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selections = birth
        .volumes()
        .load_run_selections(birth.start.run_id)
        .await
        .unwrap();
    let now = crate::authority_harness::pg_time();
    birth
        .volumes()
        .acquire_run(
            &selections,
            birth.scope.host_id(),
            now,
            now + time::Duration::hours(2),
        )
        .await
        .unwrap();
    birth
        .repository()
        .bind_resources(birth.start.run_id, None, None, None, &birth.vm.0)
        .await
        .unwrap();
    for path in [
        include_str!("../../../../../../migrations/0124_qualified_invocation_authority.sql"),
        include_str!("../../../../../../migrations/0125_qualified_invocation_empty_secret.sql"),
    ] {
        sqlx::raw_sql(path).execute(&pool).await.unwrap();
    }
    let metadata = PostgresSecretMountMetadata::new(birth.birth.worker.clone())
        .with_qualified_invocation_configuration(birth.config.clone());
    let manager = super::ports::manager(metadata.clone());
    let mut run = birth.repository().get(birth.start.run_id).await.unwrap();
    let before = birth.rows().await;
    assert!(manager.prepare(&run).await.is_err());
    assert!(manager.reauthorize(&run).await.is_err());
    run.state = RunState::Provisioning;
    assert!(
        metadata
            .verify_qualified_invocation_empty_bindings(&run)
            .await
            .is_err(),
        "caller phase cannot replace actual LeasingVolume even with complete zero set and bound VM"
    );
    assert_eq!(birth.rows().await, before);
    birth.physical.assert_unchanged();
}
