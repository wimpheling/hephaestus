#[path = "../../../heph-std/platform/recipe/postgres/tests/execution_preparation/import_faults.rs"]
mod import_faults;

pub use import_faults::{allow_revisions, fail_revisions};

use hephaestus_app::recipe_instance_import::{InstanceImportError, InstanceImportFailure};
use recipe_application::{DeploymentError, InstallProgress};
use sqlx::PgPool;

use crate::{fixtures, instance_fixture::Harness};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn real_import_success_with_wrong_recorder_role_keeps_original_claim_for_exact_reconciliation(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let claim = h.claim().await;
    let importer = h.configured(h.preparation.clone(), h.app.clone());
    assert!(matches!(
        importer
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await,
        Err(InstanceImportError::Held {
            failure: InstanceImportFailure::Completion(DeploymentError::AuthorizationDenied),
            ambiguity_error: None
        })
    ));
    assert_eq!(h.active().await, Some(claim.clone()));
    let facts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instances WHERE id=$1),
      (SELECT count(*) FROM release_command_inbox),(SELECT count(*) FROM recipe_effect_verifications)")
      .bind(h.prepared().await.instance_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        facts,
        (1, 1, 0),
        "actual committed SQL import is not a recorded completion"
    );
    let result = h
        .importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    assert!(result.active_attempt.is_none());
    let original: serde_json::Value =
        sqlx::query_scalar("SELECT claim_json FROM recipe_effect_attempts WHERE id=$1")
            .bind(claim.provenance.attempt_id.as_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        original["resource_version"],
        serde_json::json!(claim.resource_version)
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_attempts),
      (SELECT count(*) FROM release_command_inbox),(SELECT count(*) FROM recipe_execution_transitions WHERE kind='reconciled')")
      .fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (1, 1, 1));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn actual_revision_insert_failure_rolls_back_import_but_does_not_infer_absence_or_reclaim(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    fail_revisions(&pool).await;
    assert!(matches!(
        h.importer()
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await,
        Err(InstanceImportError::Held {
            failure: InstanceImportFailure::Release(_),
            ambiguity_error: None
        })
    ));
    assert_eq!(h.active().await, Some(claim.clone()));
    let counts:(i64,i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM agent_instances WHERE id=$1),
      (SELECT count(*) FROM agent_instance_revisions WHERE id=$2),(SELECT count(*) FROM release_command_inbox),
      (SELECT count(*) FROM agent_instance_events WHERE instance_id=$1)")
      .bind(prepared.instance_id().as_uuid()).bind(prepared.revision_id().as_uuid()).fetch_one(&pool).await.unwrap();
    assert_eq!(counts, (0, 0, 0, 0));
    let unproven: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipe_effect_verifications WHERE outcome='not_applied'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        unproven, 0,
        "a service error never becomes an authoritative absence receipt"
    );
    allow_revisions(&pool).await;
    let result = h
        .importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    assert_eq!(result.install, InstallProgress::Ready);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM recipe_effect_attempts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
}
