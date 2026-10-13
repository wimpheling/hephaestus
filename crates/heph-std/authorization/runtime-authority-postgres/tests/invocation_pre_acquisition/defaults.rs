use super::fixture::Fixture;
use authz_postgres::AUTHORIZATION_MODEL_VERSION;
use run_domain::RunKind;
use run_orchestrator::RunRepository;
use runtime_authority::RuntimeAuthorityError;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use runtime_types::RunId;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
#[ignore = "requires genuine private119 PostgreSQL/native Local prerequisites"]
async fn default_repository_and_other_producers_never_fall_back(pool: PgPool) {
    let f = Box::pin(Fixture::new(&pool, 0)).await;
    let before = f.rows().await;
    let default = PgRuntimeSessionRepository::new(f.birth.birth.worker.clone());
    assert!(matches!(
        default
            .verify_qualified_invocation_pre_acquisition(&f.run, AUTHORIZATION_MODEL_VERSION)
            .await,
        Err(RuntimeAuthorityError::Unsupported)
    ));
    let ordinary = run_postgres::PgRunRepository::new(f.birth.birth.worker.clone())
        .get(RunId::from_uuid(f.birth.birth.fixture.run))
        .await
        .unwrap();
    // The prerequisite seed owns a genuine Update Run, not a Normal admission.
    assert_eq!(ordinary.kind, RunKind::Update);
    assert!(
        f.repository
            .verify_qualified_invocation_pre_acquisition(&ordinary, AUTHORIZATION_MODEL_VERSION)
            .await
            .is_err()
    );
    // Supplied other-kind claims are denied without routing ordinary queries.
    for kind in [RunKind::Normal, RunKind::Update] {
        let mut run = f.run.clone();
        run.kind = kind;
        f.denied(&run).await;
    }
    assert_eq!(f.rows().await, before);
    f.allowed().await;
}
