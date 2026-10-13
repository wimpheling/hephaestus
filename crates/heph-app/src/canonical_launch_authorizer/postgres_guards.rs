use heph_run::{RunKind, RunState};
use identity_domain::RequestId;
use run_orchestrator::RunLaunchAuthorizer;
use runtime_authority_postgres::PgRuntimeSessionRepository;
use sqlx::PgPool;
use uuid::Uuid;

use super::{fixture::Fixture, scalar, wrapper};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL and explicit private profile loader"]
async fn disabled_configuration_nonworker_and_forged_original_actor_fail_closed(pool: PgPool) {
    let f = Fixture::new(pool).await;
    let configured = wrapper(&f);
    configured.authorize_with_volumes(&f.run).await.unwrap();
    let disabled = super::CanonicalRunLaunchAuthorizer::new(
        PgRuntimeSessionRepository::new(f.worker.clone()),
        scalar(&f.worker),
    );
    assert!(disabled.authorize_with_volumes(&f.run).await.is_err());
    let nonworker = super::CanonicalRunLaunchAuthorizer::new(
        PgRuntimeSessionRepository::new(f.pool.clone()).with_canonical_live_snapshots(),
        scalar(&f.pool),
    );
    assert!(nonworker.authorize_with_volumes(&f.run).await.is_err());
    let request: Uuid = sqlx::query_scalar("SELECT request_id FROM run_requests WHERE run_id=$1")
        .bind(f.run.id.as_uuid())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let original = RequestId::from_uuid(request);
    assert!(
        scalar(&f.pool)
            .authorize_canonical_source(&f.run, f.actor.maintainer, original, false)
            .await
            .is_err()
    );
    assert!(
        scalar(&f.worker)
            .authorize_canonical_source(&f.run, f.actor.owner, original, false)
            .await
            .is_err()
    );
    assert!(
        scalar(&f.worker)
            .authorize_canonical_source(&f.run, f.actor.maintainer, RequestId::new(), false)
            .await
            .is_err()
    );
    let mut substitution = f.run.clone();
    substitution.kind = RunKind::Update;
    assert!(
        configured
            .authorize_with_volumes(&substitution)
            .await
            .is_err()
    );
    assert!(f.evidence().await["session"].is_null());
    assert!(f.envelopes().is_empty());
    drop(disabled);
    drop(nonworker);
    drop(configured);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL and explicit private profile loader"]
async fn closed_preissuance_and_missing_started_session_cannot_claim_issued_authority(
    pool: PgPool,
) {
    let f = Fixture::new(pool).await;
    let authorizer = wrapper(&f);
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=now() WHERE id=$1")
        .bind(f.run.id.as_uuid())
        .execute(&f.worker)
        .await
        .unwrap();
    assert!(authorizer.authorize_with_volumes(&f.run).await.is_err());
    let request: Uuid = sqlx::query_scalar("SELECT request_id FROM run_requests WHERE run_id=$1")
        .bind(f.run.id.as_uuid())
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(
        scalar(&f.worker)
            .authorize_canonical_source(
                &f.run,
                f.actor.maintainer,
                RequestId::from_uuid(request),
                true
            )
            .await
            .is_err()
    );
    assert!(f.evidence().await["session"].is_null());
    drop(authorizer);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable PostgreSQL and explicit private profile loader"]
async fn scalar_method_delegates_without_opting_into_canonical_profile(pool: PgPool) {
    let mut f = Fixture::new(pool).await;
    f.advance(RunState::Provisioning).await;
    let scalar = scalar(&f.worker);
    scalar.authorize(&f.run).await.unwrap();
    let disabled = super::CanonicalRunLaunchAuthorizer::new(
        PgRuntimeSessionRepository::new(f.worker.clone()),
        scalar,
    );
    disabled.authorize(&f.run).await.unwrap();
    assert!(disabled.authorize_with_volumes(&f.run).await.is_err());
    assert!(f.evidence().await["session"].is_null());
    assert!(f.envelopes().is_empty());
    drop(disabled);
    f.worker.close().await;
}
