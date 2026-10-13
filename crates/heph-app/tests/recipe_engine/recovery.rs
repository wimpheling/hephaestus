use hephaestus_app::{
    recipe_engine::RecipeApplicationError,
    recipe_instance_import::{InstanceImportError, InstanceImportFailure},
    recipe_owned_volume::{OwnedVolumeCreateError, OwnedVolumeCreateFailure},
};
use recipe_application::{
    DeploymentError, DeploymentExecutionPreparationRepository, InstallProgress,
};
use sqlx::PgPool;

use super::fixture::Fixture;
use crate::fixtures;

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn ready_disk_before_original_ledger_completion_resumes_same_operation(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    assert!(
        matches!(f.with_bad_volume_recorder().resume(&f.h.actor, f.h.command, f.h.intent.id()).await,
        Err(RecipeApplicationError::Volume(error)) if matches!(*error, OwnedVolumeCreateError::Held {
            failure: OwnedVolumeCreateFailure::Completion(DeploymentError::AuthorizationDenied), ..
        }))
    );
    let original = f.stored_context().await;
    let bytes = std::fs::read(original.claim.purpose.canonical_path()).unwrap();
    let before = f.engine.inspect(&f.h.actor, f.h.intent.id()).await.unwrap();
    assert!(
        before.ledger.resources[&fixtures::key("data")]
            .active_attempt
            .is_some()
    );
    let attempt = before.ledger.resources[&fixtures::key("data")].active_attempt;
    assert_eq!(f.count("formats"), 1);
    let complete = f.resume().await.unwrap();
    assert_eq!(
        complete.ledger.resources[&fixtures::key("data")].install,
        InstallProgress::Ready
    );
    assert!(complete.activations[0].result.is_ok());
    assert_eq!(f.stored_context().await, original);
    assert_eq!(f.count("formats"), 1);
    assert_eq!(
        std::fs::read(original.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    let claim_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recipe_effect_attempts WHERE id=$1")
            .bind(attempt.unwrap().as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(claim_count, 1);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn actual_import_commit_before_original_ledger_completion_recovers_same_command(
    pool: PgPool,
) {
    let f = Fixture::new(&pool).await;
    assert!(
        matches!(f.with_bad_instance_recorder().resume(&f.h.actor, f.h.command, f.h.intent.id()).await,
        Err(RecipeApplicationError::Instance(error)) if matches!(*error, InstanceImportError::Held {
            failure: InstanceImportFailure::Completion(DeploymentError::AuthorizationDenied), ..
        }))
    );
    let frozen = f.h.prepared().await;
    let active = f.h.active().await.unwrap();
    let pending:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instances WHERE id=$1 AND NOT run_gate_open),(SELECT count(*) FROM agent_instance_execution_activations WHERE instance_id=$1)")
        .bind(frozen.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        pending,
        (1, 0),
        "actual import is not completion or activation"
    );
    let formats = f.count("formats");
    let fscks = f.count("fscks");
    let complete = f.resume().await.unwrap();
    assert!(complete.activations[0].result.is_ok());
    assert_eq!(f.count("formats"), formats);
    assert_eq!(f.count("fscks"), fscks);
    let exact:(i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_attempts WHERE deployment_id=$1 AND resource_name='sqlite'),(SELECT count(*) FROM release_command_inbox WHERE command_key=$2)")
        .bind(f.h.intent.id().as_uuid()).bind(frozen.command_key().as_bytes().as_slice()).fetch_one(&pool).await.unwrap();
    assert_eq!(exact, (1, 1));
    let original: serde_json::Value =
        sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id=$1")
            .bind(active.provenance.attempt_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        original["resource_version"],
        serde_json::json!(active.resource_version)
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn withdrawn_source_keeps_ready_uncompleted_birth_held_and_inspectable(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    assert!(
        f.with_bad_volume_recorder()
            .resume(&f.h.actor, f.h.command, f.h.intent.id())
            .await
            .is_err()
    );
    let original = f.stored_context().await;
    let bytes = std::fs::read(original.claim.purpose.canonical_path()).unwrap();
    f.withdraw_source().await;
    let before = f.projection().await;
    assert!(matches!(
        f.resume().await,
        Err(RecipeApplicationError::Deployment(
            DeploymentError::AuthorizationDenied
        ))
    ));
    let inspection = f.engine.inspect(&f.h.actor, f.h.intent.id()).await.unwrap();
    assert!(
        inspection.ledger.resources[&fixtures::key("data")]
            .active_attempt
            .is_some()
    );
    assert!(
        inspection.preparation.is_some(),
        "historical inspection does not borrow source authority"
    );
    assert_eq!(f.projection().await, before);
    assert_eq!(
        std::fs::read(original.claim.purpose.canonical_path()).unwrap(),
        bytes
    );
    assert_eq!(f.count("formats"), 1);
    let wrong:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM volume_owned_partial_receipts)+(SELECT count(*) FROM recipe_effect_verifications WHERE outcome='not_applied')")
        .fetch_one(&pool).await.unwrap();
    assert_eq!(wrong, 0, "Ready is not partial custody or absence");
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn configured_policy_drift_and_borrowed_actor_cannot_take_effects(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    f.h.preparation
        .prepare_execution(&f.h.actor, f.h.command, f.h.intent.id())
        .await
        .unwrap();
    let before = f.projection().await;
    assert!(matches!(
        f.drifted()
            .resume(&f.h.actor, f.h.command, f.h.intent.id())
            .await,
        Err(RecipeApplicationError::Deployment(
            DeploymentError::InputConflict
        ))
    ));
    let outsider = crate::support::identity(f.h.fixture.outsider);
    assert!(matches!(
        f.engine
            .resume(&outsider, f.h.command, f.h.intent.id())
            .await,
        Err(RecipeApplicationError::Deployment(
            DeploymentError::InputConflict
        ))
    ));
    assert_eq!(f.projection().await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), (0, 0));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires private114 PostgreSQL and native e2fsprogs"]
async fn genuinely_admitted_remove_prevents_original_install_resume(pool: PgPool) {
    let f = Fixture::new(&pool).await;
    let snapshot =
        f.h.repository
            .inspect(&f.h.actor, f.h.intent.id())
            .await
            .unwrap();
    f.h.repository
        .admit_remove(
            &f.h.actor,
            fixtures::remove(&f.h.actor, f.h.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let before = f.projection().await;
    assert!(f.resume().await.is_err());
    assert_eq!(f.projection().await, before);
    assert_eq!((f.count("formats"), f.count("fscks")), (0, 0));
    let inspection = f.engine.inspect(&f.h.actor, f.h.intent.id()).await.unwrap();
    assert_eq!(
        inspection.ledger.lifecycle,
        recipe_application::DeploymentLifecycle::Removing
    );
    assert!(inspection.preparation.is_none());
}
