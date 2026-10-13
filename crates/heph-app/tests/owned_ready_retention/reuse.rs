use crate::{fixture::Fixture, fixtures};
use recipe_application::{
    AttemptProvenance, DeploymentAttemptId, DeploymentExecutionPreparationRepository, DeploymentId,
    EffectClaim, EffectEvidence, EffectOutcome, OwnedReadyRetentionRepository,
    ReopenRetainedOwnedVolume, ResourceAction, ResourceEffect,
};
use recipe_postgres::{PostgresEffectVerificationRecorder, VerifiedOutcome};
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn completed_birth_retains_then_explicit_fresh_external_reopening_preserves_old_receipts(
    pool: PgPool,
) {
    let f = Fixture::new(&pool, true).await;
    let original = f.original().await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.coordinator()
        .retain(&identity, &request, &f.ready.claim)
        .await
        .unwrap();
    let bytes = std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap();
    let fscks = f.birth.count("fscks");
    let (reuse_identity, claim) = external_claim(&f, identity.user_id).await;
    let reopening = ReopenRetainedOwnedVolume {
        claim: claim.clone(),
        retention_operation_id: request.operation_id,
    };
    f.actor
        .reopen_retained_owned_volume(&reuse_identity, &reopening)
        .await
        .unwrap();
    let before = f.snapshot().await;
    f.actor
        .reopen_retained_owned_volume(&reuse_identity, &reopening)
        .await
        .unwrap();
    assert_eq!(
        f.snapshot().await,
        before,
        "same active claim replays its exact immutable reopening"
    );
    let recorder = PostgresEffectVerificationRecorder::new(f.birth.h.worker.clone());
    recorder
        .record(
            &claim,
            claim.provenance.attempt_id,
            VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    f.birth
        .h
        .repository
        .complete_resource_effect(
            &reuse_identity,
            &claim,
            EffectEvidence {
                identity: claim.identity,
                input_hash: claim.input_hash,
                outcome: EffectOutcome::Applied,
            },
        )
        .await
        .unwrap();
    let snapshot = f.snapshot().await;
    f.actor
        .reopen_retained_owned_volume(&reuse_identity, &reopening)
        .await
        .unwrap();
    assert_eq!(
        f.snapshot().await,
        snapshot,
        "completed external replay grants no new IO"
    );
    assert!(
        f.actor
            .begin_owned_ready_retention(&identity, &request)
            .await
            .unwrap()
            .completed
            .is_some()
    );
    assert!(
        f.birth
            .local
            .verify_owned_ready(&f.ready.claim)
            .await
            .is_err()
    );
    assert_eq!(f.original().await, original);
    assert_eq!(
        std::fs::read(f.ready.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!(
        (f.birth.count("formats"), f.birth.count("fscks")),
        (1, fscks)
    );
    assert_completed_reuse(&f).await;
    drop(f);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private116 PostgreSQL and native e2fsprogs"]
async fn unfinished_retention_keeps_positive_fact_but_external_reuse_remains_held(pool: PgPool) {
    let f = Fixture::new(&pool, false).await;
    let (identity, request) = f.remove(f.birth.h.actor.user_id).await;
    f.coordinator()
        .retain(&identity, &request, &f.ready.claim)
        .await
        .unwrap();
    let (reuse_identity, claim) = external_claim(&f, identity.user_id).await;
    let before = f.snapshot().await;
    assert!(
        f.actor
            .reopen_retained_owned_volume(
                &reuse_identity,
                &ReopenRetainedOwnedVolume {
                    claim,
                    retention_operation_id: request.operation_id,
                }
            )
            .await
            .is_err()
    );
    assert_eq!(f.snapshot().await, before);
    let counts:(i64,i64,bool)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_owned_ready_retentions),(SELECT count(*) FROM recipe_owned_ready_reopenings),recipe_owned_first_birth_completed($1)")
        .bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 0, false));
    drop(f);
}

async fn external_claim(
    f: &Fixture,
    user: identity_domain::UserId,
) -> (identity_domain::AuthenticatedIdentity, EffectClaim) {
    let identity = crate::support::identity(user);
    let mut seeded = f.birth.h.fixture;
    seeded.volume = f
        .ready
        .claim
        .purpose
        .receipt()
        .intent
        .registration()
        .id()
        .as_uuid();
    // Immutable recipe definition identity cannot be reused for owned->external shape.
    let source = fixtures::source(&seeded, true)
        .replace("recipe_id = \"database\"", "recipe_id = \"database-reuse\"");
    let intent = fixtures::build(
        &seeded,
        &source,
        DeploymentId::from_uuid(Uuid::new_v4()).unwrap(),
        "explicit-ready-reuse",
        &BTreeMap::new(),
        &[fixtures::catalog(&seeded)],
        true,
    );
    let install = fixtures::install(&identity, intent.clone());
    let command = install.command;
    f.birth
        .h
        .repository
        .admit_install(&identity, install)
        .await
        .unwrap();
    f.birth
        .h
        .preparation
        .prepare_execution(&identity, command, intent.id())
        .await
        .unwrap();
    let snapshot = f
        .birth
        .h
        .repository
        .inspect(&identity, intent.id())
        .await
        .unwrap();
    let claim = f
        .birth
        .h
        .repository
        .claim_resource_effect(
            &identity,
            ResourceEffect {
                command,
                deployment_id: intent.id(),
                resource: fixtures::key("data"),
                expected_deployment_version: snapshot.version,
                expected_resource_version: snapshot.resources[&fixtures::key("data")].version,
                action: ResourceAction::VerifyExternal,
                provenance: AttemptProvenance::new(
                    &identity,
                    DeploymentAttemptId::from_uuid(Uuid::new_v4()).unwrap(),
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    (identity, claim)
}

async fn assert_completed_reuse(f: &Fixture) {
    let (closed,reopened,fence,first_complete):(bool,bool,i64,bool)=sqlx::query_as("SELECT ready_retention_closure_id IS NOT NULL,ready_retention_reopen_id IS NOT NULL,provisioning_generation,recipe_owned_first_birth_completed(id) FROM agent_instance_state_volumes WHERE id=$1")
        .bind(f.ready.claim.purpose.receipt().intent.registration().id().as_uuid()).fetch_one(&f.pool).await.unwrap();
    assert!(closed && reopened && first_complete);
    assert_eq!(fence, 1);
    let named: bool = sqlx::query_scalar("SELECT named_volume_dispatch_supported($1)")
        .bind(f.birth.h.fixture.instance)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(
        !named,
        "explicit reuse does not enable ordinary named dispatch policy"
    );
}
