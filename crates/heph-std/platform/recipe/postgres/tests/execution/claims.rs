use crate::{fixtures, harness::Harness};
use recipe_application::{DeploymentError, ResourceAction};
use sqlx::PgPool;

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn concurrent_claims_fence_one_attempt_and_reused_attempt_requires_reconciliation(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false, false).await;
    let identity = h.request();
    let first = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let second_identity = h.request();
    let mut second = first.clone();
    second.provenance =
        recipe_application::AttemptProvenance::new(&second_identity, crate::harness::attempt())
            .expect("attempt");
    let (a, b) = tokio::join!(
        h.repository.claim_resource_effect(&identity, first.clone()),
        h.repository
            .claim_resource_effect(&second_identity, second.clone())
    );
    let claim = match (a, b) {
        (Ok(claim), Err(DeploymentError::StaleClaim))
        | (Err(DeploymentError::StaleClaim), Ok(claim)) => claim,
        pair => panic!("unexpected CAS results {pair:?}"),
    };
    assert_eq!(claim.generation, 1);
    assert_eq!(claim.resource_version, 1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM recipe_effect_attempts")
        .fetch_one(&pool)
        .await
        .expect("claims");
    assert_eq!(count, 1);
    let (winner_identity, winner_request) =
        if claim.provenance.attempt_id == first.provenance.attempt_id {
            (&identity, first)
        } else {
            (&second_identity, second)
        };
    assert!(matches!(
        h.repository
            .claim_resource_effect(winner_identity, winner_request)
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    let snapshot = h
        .repository
        .inspect(&h.identity, h.intent.id())
        .await
        .expect("committed claim");
    assert_eq!(snapshot.version, 1);
    assert_eq!(
        snapshot.resources[&fixtures::key("data")].active_attempt,
        Some(claim.provenance.attempt_id)
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn install_dependencies_and_external_mutation_are_closed(pool: PgPool) {
    let h = Harness::new(&pool, true, false).await;
    let identity = h.request();
    let premature = h
        .effect(&identity, h.command, "sqlite", ResourceAction::Create)
        .await;
    assert!(matches!(
        h.repository
            .claim_resource_effect(&identity, premature)
            .await,
        Err(DeploymentError::InvalidAction)
    ));
    h.apply(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let identity = h.request();
    h.apply(&identity, h.command, "sqlite", ResourceAction::Create)
        .await;
    let snapshot = h
        .repository
        .inspect(&identity, h.intent.id())
        .await
        .expect("ready graph");
    assert!(
        snapshot
            .resources
            .values()
            .all(|p| p.install == recipe_application::InstallProgress::Ready)
    );
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn external_volume_can_only_be_verified_and_is_retained(pool: PgPool) {
    let h = Harness::new(&pool, false, true).await;
    let identity = h.request();
    let mutate = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    assert!(matches!(
        h.repository.claim_resource_effect(&identity, mutate).await,
        Err(DeploymentError::InvalidAction)
    ));
    h.apply(&identity, h.command, "data", ResourceAction::VerifyExternal)
        .await;
    let snapshot = h
        .repository
        .inspect(&identity, h.intent.id())
        .await
        .expect("external");
    assert_eq!(
        snapshot.resources[&fixtures::key("data")].removal,
        recipe_application::RemovalProgress::Retained
    );
    assert_eq!(
        snapshot.intent.resources()[&fixtures::key("data")].identity(),
        h.intent.resources()[&fixtures::key("data")].identity()
    );
}
