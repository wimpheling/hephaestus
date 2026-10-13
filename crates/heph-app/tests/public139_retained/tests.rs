use super::{
    atomic,
    fixture::Fixture,
    lifecycle,
    pins::{self, Pins},
};
use hephaestus_app::recipe_retained_volume::{
    PreparedRetainedVolumeVerifier, RetainedVolumeVerifier,
};
use recipe_application::{
    OwnedReadyRetentionRepository, ReopenRetainedOwnedVolume, RetainedExternalCompletion,
};
use sqlx::PgPool;
use std::os::unix::fs::MetadataExt;
use volume_trait::VolumeRetainedReuseRepository;

// Consuming Fixture::close(self).await retains native custody through pool shutdown.
// Rust 1.88 incorrectly suggests dropping the already moved fixture afterward.
#[allow(clippy::significant_drop_tightening)]
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL and actual configured libkrun/KVM/cgroup/e2fsprogs"]
async fn genuine_retained_chain_checks_each_pin_and_authorized_reopen_closes_old_context(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    assert_eq!(f.target.action, recipe_application::ResourceAction::Create);
    let bytes = backing(&f);
    let genuine = Pins::genuine(&f);
    pins::check(&f.core.worker, &genuine, true).await;
    for slot in 0..7 {
        let mut wrong = genuine.clone();
        wrong.substitute(slot);
        pins::check(&f.core.worker, &wrong, false).await;
    }
    pins::old121_rejects(&f.core.worker, &genuine).await;
    let (identity, claim) = lifecycle::external(&f.core, &f.original).await;
    f.core
        .actor
        .reopen_retained_owned_volume(
            &identity,
            &ReopenRetainedOwnedVolume {
                claim,
                retention_operation_id: recipe_application::DeploymentAttemptId::from_uuid(
                    f.retained.retention_operation_id,
                )
                .unwrap(),
            },
        )
        .await
        .unwrap();
    pins::check(&f.core.worker, &genuine, false).await;
    assert!(
        f.core
            .retention_worker
            .retained_reuse_context(
                &f.original.claim,
                f.retained.retained.closure.fields().id,
                f.retained.retention_operation_id
            )
            .await
            .is_err()
    );
    assert_eq!(backing(&f), bytes);
    f.close().await;
}

// Consuming Fixture::close(self).await retains native custody through pool shutdown.
// Rust 1.88 incorrectly suggests dropping the already moved fixture afterward.
#[allow(clippy::significant_drop_tightening)]
#[sqlx::test(migrations = false)]
#[ignore = "requires disposable public139 PostgreSQL and actual configured libkrun/KVM/cgroup/e2fsprogs"]
async fn paired_actor_rollback_completion_and_replay_recheck_current_authority(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let bytes = backing(&f);
    let (identity, claim) = lifecycle::external(&f.core, &f.original).await;
    let plan = f
        .core
        .actor
        .prepare_retained_external_verification(&identity, &claim)
        .await
        .unwrap();
    let mut custody = f
        .core
        .native
        .local
        .observe_retained_owned_reuse(f.core.retention_worker.as_ref(), &f.retained)
        .await
        .unwrap();
    custody.revalidate().unwrap();
    f.core
        .recorder
        .record(
            &claim,
            claim.provenance.attempt_id,
            recipe_postgres::VerifiedOutcome::Applied,
        )
        .await
        .unwrap();
    custody.revalidate().unwrap();
    // A genuine Applied worker receipt exists for these denials; missing proof
    // cannot mask a failure to recheck current authority or exact claim input.
    deny_pending(&f, &identity, &plan).await;
    custody.revalidate().unwrap();
    atomic::force_completion_rollback(&f.core, &identity, &plan).await;
    custody.revalidate().unwrap();
    drop(custody);
    let verifier = PreparedRetainedVolumeVerifier::new(
        f.core.actor.clone(),
        f.core.retention_worker.clone(),
        f.core.metadata.clone(),
        f.core.recorder.clone(),
        f.core.repository.clone(),
        f.core.native.local.clone(),
        f.core.native.owner.clone(),
        f.core.native.provider.clone(),
    )
    .unwrap();
    let progress = verifier
        .verify_retained_volume(&identity, &claim)
        .await
        .unwrap();
    assert_eq!(progress.install, recipe_application::InstallProgress::Ready);
    assert!(progress.active_attempt.is_none());
    assert!(verifier.pending_verifications().is_empty());
    assert_pair(&f, &claim).await;
    let committed = atomic::snapshot(&f.core, &claim).await;
    assert_eq!(
        verifier
            .verify_retained_volume(&identity, &claim)
            .await
            .unwrap(),
        progress
    );
    assert_eq!(atomic::snapshot(&f.core, &claim).await, committed);
    manager(&f, false).await;
    let denied_replay = atomic::snapshot(&f.core, &claim).await;
    assert!(
        verifier
            .verify_retained_volume(&identity, &claim)
            .await
            .is_err()
    );
    assert_eq!(atomic::snapshot(&f.core, &claim).await, denied_replay);
    manager(&f, true).await;
    assert_eq!(
        verifier
            .verify_retained_volume(&identity, &claim)
            .await
            .unwrap(),
        progress
    );
    assert_eq!(backing(&f), bytes);
    drop(verifier);
    f.close().await;
}

async fn deny_pending(
    f: &Fixture,
    identity: &identity_domain::AuthenticatedIdentity,
    plan: &recipe_application::RetainedExternalVerificationPlan,
) {
    let claim = &plan.claim;
    status(f, "suspended").await;
    let denied_before = atomic::snapshot(&f.core, claim).await;
    assert!(
        f.core
            .actor
            .complete_retained_external_verification(
                identity,
                plan,
                RetainedExternalCompletion::Completed
            )
            .await
            .is_err()
    );
    assert_eq!(atomic::snapshot(&f.core, claim).await, denied_before);
    status(f, "active").await;
    let before = atomic::snapshot(&f.core, claim).await;
    let mut stale = plan.clone();
    stale.claim.resource_version += 1;
    assert!(
        f.core
            .actor
            .complete_retained_external_verification(
                identity,
                &stale,
                RetainedExternalCompletion::Completed
            )
            .await
            .is_err()
    );
    assert_eq!(atomic::snapshot(&f.core, claim).await, before);
}

async fn status(f: &Fixture, value: &str) {
    sqlx::query("UPDATE users SET status=$1 WHERE id=$2")
        .bind(value)
        .bind(f.core.identity.user_id.as_uuid())
        .execute(&f.core.admin)
        .await
        .unwrap();
}

async fn manager(f: &Fixture, present: bool) {
    // Actual FGA relationship drift; the authenticated user remains active and
    // has no organization owner/admin relationship that could mask this denial.
    if present {
        sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
            .bind(f.core.project.as_uuid())
            .bind(f.core.identity.user_id.as_uuid())
            .execute(&f.core.admin)
            .await
            .unwrap();
    } else {
        sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
            .bind(f.core.project.as_uuid())
            .bind(f.core.identity.user_id.as_uuid())
            .execute(&f.core.admin)
            .await
            .unwrap();
    }
}

async fn assert_pair(f: &Fixture, claim: &recipe_application::EffectClaim) {
    let counts: (i64, i64, bool) = sqlx::query_as(
        "SELECT
      (SELECT count(*) FROM recipe_owned_ready_reopenings WHERE attempt_id=$1),
      (SELECT count(*) FROM recipe_execution_transitions WHERE attempt_id=$1 AND kind='completed'),
      EXISTS(SELECT 1 FROM agent_instance_state_volumes volume
        JOIN recipe_owned_ready_reopenings reopened ON reopened.id=volume.ready_retention_reopen_id
        WHERE volume.id=$2 AND volume.ready_retention_closure_id=$3
          AND reopened.closure_id=$3 AND reopened.attempt_id=$1)",
    )
    .bind(claim.provenance.attempt_id.as_uuid())
    .bind(
        f.original
            .claim
            .purpose
            .receipt()
            .intent
            .registration()
            .id()
            .as_uuid(),
    )
    .bind(f.retained.retained.closure.fields().id)
    .fetch_one(&f.core.admin)
    .await
    .unwrap();
    assert_eq!(counts, (1, 1, true));
}

fn backing(f: &Fixture) -> (u64, u64, release_domain::ContentHash) {
    let path = f.original.claim.purpose.canonical_path();
    let meta = std::fs::metadata(&path).unwrap();
    (
        meta.dev(),
        meta.ino(),
        release_domain::ContentHash::digest(&std::fs::read(path).unwrap()),
    )
}
