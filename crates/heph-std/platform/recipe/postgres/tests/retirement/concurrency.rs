use recipe_application::{AttemptProvenance, DeploymentError};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use sqlx::PgPool;

use crate::{
    harness::{self, Harness},
    helpers,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn concurrent_retirements_have_one_winner_and_late_completion_or_worker_proof_cannot_resurrect(
    pool: PgPool,
) {
    crate::private_migrations::checkpoint(109)
        .run(&pool)
        .await
        .expect("explicit private109 checkpoint");
    let h = Harness::new(&pool, false, true).await;
    let target = helpers::claim(&h, true).await;
    let cleanup = crate::support::identity(h.fixture.maintainer);
    let first = helpers::admit_cleanup(&h, &cleanup, &target).await;
    let mut other_identity = cleanup.clone();
    other_identity.request_id = identity_domain::RequestId::new();
    let mut second = first.clone();
    second.provenance =
        AttemptProvenance::new(&other_identity, harness::attempt()).expect("another operation");
    let (a, b) = tokio::join!(
        h.repository.retire_external_observation(&cleanup, &first),
        h.repository
            .retire_external_observation(&other_identity, &second)
    );
    let winner = match (a, b) {
        (Ok(state), Err(DeploymentError::StaleClaim))
        | (Err(DeploymentError::StaleClaim), Ok(state)) => state,
        pair => panic!("unexpected retirement CAS results {pair:?}"),
    };
    assert_eq!(winner.active_attempt, None);
    assert!(matches!(
        h.repository
            .complete_resource_effect(&h.identity, &target, harness::evidence(&target))
            .await,
        Err(DeploymentError::StaleClaim)
    ));
    assert!(
        PostgresEffectVerificationRecorder::new(h.worker.clone())
            .record(&target, harness::attempt(), VerifiedOutcome::Applied)
            .await
            .is_err(),
        "late receipt insertion must fail before any provider fact is stored"
    );
    let counts: (i64, i64) = sqlx::query_as("SELECT (SELECT count(*) FROM recipe_execution_transitions WHERE kind='observation_retired'),
        (SELECT count(*) FROM recipe_effect_verifications)")
        .fetch_one(&pool).await.expect("exactly one ledger retirement and no provider evidence");
    assert_eq!(counts, (1, 0));
    assert_eq!(
        helpers::state(&pool, &target).await["generation"],
        serde_json::json!(target.generation)
    );
}
