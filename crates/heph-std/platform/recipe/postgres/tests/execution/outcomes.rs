use recipe_application::{
    AttemptProvenance, DeploymentError, DiagnosticCode, EffectOutcome, InstallProgress,
    ReconciledOutcome, ResourceAction,
};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use sqlx::PgPool;

use crate::{
    fixtures,
    harness::{self, Harness},
};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn caller_evidence_cannot_forge_success_or_modify_the_committed_claim(pool: PgPool) {
    let h = Harness::new(&pool, false, false).await;
    let identity = h.request();
    let effect = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let claim = h
        .repository
        .claim_resource_effect(&identity, effect)
        .await
        .expect("claim");
    assert!(matches!(
        h.repository
            .complete_resource_effect(&identity, &claim, harness::evidence(&claim))
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    assert!(matches!(
        PostgresEffectVerificationRecorder::new(h.app.clone())
            .record(
                &claim,
                claim.provenance.attempt_id,
                VerifiedOutcome::Applied
            )
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    let mut forged = claim.clone();
    forged.generation += 1;
    assert!(matches!(
        h.repository
            .complete_resource_effect(&identity, &forged, harness::evidence(&forged))
            .await,
        Err(DeploymentError::InputConflict)
    ));
    h.proof(
        &claim,
        claim.provenance.attempt_id,
        VerifiedOutcome::Applied,
    )
    .await;
    let completed = h
        .repository
        .complete_resource_effect(&identity, &claim, harness::evidence(&claim))
        .await
        .expect("verified success");
    let replay = h
        .repository
        .complete_resource_effect(&h.request(), &claim, harness::evidence(&claim))
        .await
        .expect("stable replay");
    assert_eq!(completed, replay);
    let mut changed = harness::evidence(&claim);
    changed.outcome = EffectOutcome::Failed(DiagnosticCode::ProviderFailure);
    assert!(matches!(
        h.repository
            .complete_resource_effect(&h.request(), &claim, changed)
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let counts: (i64, i64) = sqlx::query_as(
        "SELECT (SELECT count(*) FROM recipe_effect_verifications),
        (SELECT count(*) FROM recipe_execution_transitions WHERE kind='completed')",
    )
    .fetch_one(&pool)
    .await
    .expect("immutable outcomes");
    assert_eq!(counts, (1, 1));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn ambiguity_requires_fresh_verified_absence_before_retry_and_fences_old_completion(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false, false).await;
    let identity = h.request();
    let effect = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let claim = h
        .repository
        .claim_resource_effect(&identity, effect)
        .await
        .expect("claim");
    let ambiguous = h
        .repository
        .mark_resource_ambiguous(&identity, &claim, DiagnosticCode::ProviderOutcomeUnknown)
        .await
        .expect("ambiguity durable");
    assert_eq!(ambiguous.active_attempt, Some(claim.provenance.attempt_id));
    let retry = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    assert!(matches!(
        h.repository.claim_resource_effect(&identity, retry).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    let observation = h.request();
    let provenance =
        AttemptProvenance::new(&observation, harness::attempt()).expect("fresh observation");
    assert!(matches!(
        h.repository
            .record_reconciliation(
                &observation,
                h.command,
                &claim,
                provenance,
                ReconciledOutcome::NotApplied
            )
            .await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    h.proof(&claim, provenance.attempt_id, VerifiedOutcome::NotApplied)
        .await;
    let absent = h
        .repository
        .record_reconciliation(
            &observation,
            h.command,
            &claim,
            provenance,
            ReconciledOutcome::NotApplied,
        )
        .await
        .expect("verified absence");
    assert_eq!(absent.install, InstallProgress::Failed);
    assert_eq!(absent.active_attempt, None);
    let retry_identity = h.request();
    let retry = h
        .effect(&retry_identity, h.command, "data", ResourceAction::Create)
        .await;
    let next = h
        .repository
        .claim_resource_effect(&retry_identity, retry)
        .await
        .expect("fresh generation");
    assert_eq!(next.generation, claim.generation + 1);
    assert_eq!(next.identity, claim.identity);
    assert!(matches!(
        h.repository
            .complete_resource_effect(&identity, &claim, harness::evidence(&claim))
            .await,
        Err(DeploymentError::StaleClaim)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn definite_failure_and_unproven_reconciliation_keep_creation_blocked(pool: PgPool) {
    let h = Harness::new(&pool, false, false).await;
    let identity = h.request();
    let effect = h
        .effect(&identity, h.command, "data", ResourceAction::Create)
        .await;
    let claim = h
        .repository
        .claim_resource_effect(&identity, effect)
        .await
        .expect("claim");
    h.proof(
        &claim,
        claim.provenance.attempt_id,
        VerifiedOutcome::Failed(DiagnosticCode::ProviderFailure),
    )
    .await;
    let mut failed = harness::evidence(&claim);
    failed.outcome = EffectOutcome::Failed(DiagnosticCode::ProviderFailure);
    let progress = h
        .repository
        .complete_resource_effect(&identity, &claim, failed)
        .await
        .expect("failure");
    assert_eq!(progress.install, InstallProgress::Failed);
    assert_eq!(progress.active_attempt, Some(claim.provenance.attempt_id));
    let observer = h.request();
    let provenance = AttemptProvenance::new(&observer, harness::attempt()).expect("observation");
    let unproven = ReconciledOutcome::Unproven(DiagnosticCode::FencingUnproven);
    let uncertain = h
        .repository
        .record_reconciliation(&observer, h.command, &claim, provenance, unproven)
        .await
        .expect("conservative observation");
    assert_eq!(uncertain.install, InstallProgress::RecoveryRequired);
    assert_eq!(
        h.repository
            .record_reconciliation(&observer, h.command, &claim, provenance, unproven)
            .await
            .expect("replay"),
        uncertain
    );
    assert!(matches!(
        h.repository
            .record_reconciliation(
                &observer,
                h.command,
                &claim,
                provenance,
                ReconciledOutcome::NotApplied
            )
            .await,
        Err(DeploymentError::InputConflict)
    ));
    let retry = h
        .effect(&observer, h.command, "data", ResourceAction::Create)
        .await;
    assert!(matches!(
        h.repository.claim_resource_effect(&observer, retry).await,
        Err(DeploymentError::ReconciliationRequired)
    ));
    let snapshot = h
        .repository
        .inspect(&observer, h.intent.id())
        .await
        .expect("blocked state");
    assert_eq!(snapshot.resources[&fixtures::key("data")], uncertain);
}
