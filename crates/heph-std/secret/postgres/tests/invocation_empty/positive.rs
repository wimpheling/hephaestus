use super::{fixture::Fixture, ports};
use capability_domain::RuntimeCredentialGeneration;
use run_domain::RunState;
use run_orchestrator::{RunRepository, RunSecretManager};
use runtime_authority::RuntimeSessionRepository;
use secret_postgres::PostgresSecretMountMetadata;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn saved_empty_zero_volumes_never_dispatch_and_default_stays_unsupported(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    assert!(f.verify().await);
    f.allowed_unchanged().await;
    let run = f.run().await;
    let before = f.rows().await;
    let default = ports::manager(PostgresSecretMountMetadata::new(
        f.authority.birth.birth.worker.clone(),
    ));
    assert!(default.prepare(&run).await.is_err());
    assert!(default.reauthorize(&run).await.is_err());
    assert_eq!(f.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 B1 PostgreSQL/Local prerequisites"]
async fn two_real_volumes_verify_before_ceiling_and_through_acknowledged_late_phases(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 2)).await;
    f.allowed_unchanged().await;
    let pending = f.authority.issue().await;
    f.allowed_unchanged().await;
    f.authority
        .repository
        .acknowledge(
            pending.id,
            RuntimeCredentialGeneration::INITIAL,
            crate::authority_harness::pg_time(),
        )
        .await
        .unwrap();
    f.authority.attach_metadata().await;
    for phase in [RunState::Starting, RunState::Running] {
        f.authority
            .birth
            .repository()
            .transition(f.authority.birth.start.run_id, phase, None, None)
            .await
            .unwrap();
        f.allowed_unchanged().await;
    }
    f.authority
        .birth
        .birth
        .assert_historical_migrations(&pool)
        .await;
}
