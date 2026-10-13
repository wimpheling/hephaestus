use run_domain::FreshRunExecutionProfile;
use run_postgres::{
    PgQualifiedInvocationRoutingReader, QualifiedOrdinaryEvidence,
    QualifiedOrdinaryRoutingDecision as Decision, QualifiedOrdinaryRoutingHoldReason as Hold,
};
use runtime_types::AgentInstanceRevisionId;
use sqlx::PgPool;

use super::fixture::{self, Fixture};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn genuine_unborn_request_becomes_exact_consumed_legacy_original(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    for observed in [
        f.reader.inspect_ordinary_start(&f.start).await.unwrap(),
        f.reader
            .inspect_ordinary_original(f.start.run_id)
            .await
            .unwrap(),
    ] {
        let Decision::ProspectivelyUnborn(unborn) = observed else {
            panic!("real enriched outbox must qualify unborn: {observed:?}");
        };
        assert_eq!(unborn.start(), &f.start);
        assert_eq!(unborn.source().producer_request_id(), f.producer);
        assert_eq!(unborn.source().receive_id(), Some(f.receive));
    }
    f.birth().await;
    let original = f.original().await;
    assert_eq!(original.original_start(), &f.start);
    assert_eq!(original.profile(), FreshRunExecutionProfile::LegacyScalar);
    assert_eq!(
        original.evidence(),
        QualifiedOrdinaryEvidence::LegacyProvisionConsumed
    );
    assert_eq!(original.owner(), f.scope.provider_scope());
    assert_eq!(original.vm_id().0, f.start.run_id.to_string());
    assert_eq!(original.source().repository_id(), f.repository);
    assert_eq!(original.source().commit_sha(), f.commit);
    assert_eq!(original.source().git_ref(), "refs/heads/main");
    assert_eq!(original.source().original_actor(), None);
    assert_eq!(original.source().original_request(), None);
    assert_eq!(original.release_contract_hash(), &[4_u8; 32]);
    assert_eq!(
        fixture::qualified(f.reader.inspect_ordinary_start(&f.start).await.unwrap()),
        original
    );
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn application_and_owner_pools_cannot_qualify_worker_originals(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    for actual in [f.application.clone(), pool.clone()] {
        let reader = PgQualifiedInvocationRoutingReader::new(
            actual,
            fixture::configuration(&f.scope, "test/v1"),
        );
        assert_eq!(
            reader.inspect_ordinary_start(&f.start).await.unwrap(),
            Decision::Held(Hold::WorkerRoleRequired)
        );
        assert_eq!(
            reader
                .inspect_ordinary_original(f.start.run_id)
                .await
                .unwrap(),
            Decision::Held(Hold::WorkerRoleRequired)
        );
    }
    f.original().await;
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn caller_start_mismatch_is_held_before_and_after_real_birth(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let mut changed = f.start.clone();
    changed.instance_revision_id = AgentInstanceRevisionId::new();
    assert_eq!(
        f.reader.inspect_ordinary_start(&changed).await.unwrap(),
        Decision::Held(Hold::ContradictoryOriginal)
    );
    f.birth().await;
    assert_eq!(
        f.reader.inspect_ordinary_start(&changed).await.unwrap(),
        Decision::Held(Hold::ContradictoryOriginal)
    );
    f.original().await;
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn global_original_with_foreign_owner_is_held_without_fallback(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let foreign = fixture::owner();
    let reader = PgQualifiedInvocationRoutingReader::new(
        f.worker.clone(),
        fixture::configuration(&foreign, "test/v1"),
    );
    assert_eq!(
        reader.inspect_ordinary_start(&f.start).await.unwrap(),
        Decision::Held(Hold::ForeignOwner)
    );
    assert_eq!(
        reader
            .inspect_ordinary_original(f.start.run_id)
            .await
            .unwrap(),
        Decision::Held(Hold::ForeignOwner)
    );
    f.original().await;
    f.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL and local Git"]
async fn later_platform_configuration_does_not_rewrite_original_family(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.birth().await;
    let reader = PgQualifiedInvocationRoutingReader::new(
        f.worker.clone(),
        fixture::configuration(&f.scope, "future/v2"),
    );
    let original = fixture::qualified(
        reader
            .inspect_ordinary_original(f.start.run_id)
            .await
            .unwrap(),
    );
    assert_eq!(original, f.original().await);
    assert_eq!(original.source().platform_policy_version(), "test/v1");
    f.close().await;
}
