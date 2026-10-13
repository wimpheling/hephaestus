use std::sync::Arc;

use hephaestus_app::recipe_instance_import::InstanceImportError;
use identity_domain::RequestId;
use recipe_application::{DeploymentError, DeploymentExecutionProfile};
use recipe_postgres::PostgresExecutionPreparationRepository;
use sqlx::PgPool;

use crate::{
    fixtures,
    instance_fixture::{self, Harness},
    support,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn source_withdrawal_denies_first_call_and_completed_replay_before_new_effects(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    h.prepared().await;
    withdraw_source(&pool, &h).await;
    let before = instance_fixture::state(&pool).await;
    assert_denied(&h).await;
    assert_eq!(before, instance_fixture::state(&pool).await);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    h.importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
    withdraw_source(&pool, &h).await;
    let completed = instance_fixture::state(&pool).await;
    assert_denied(&h).await;
    assert_eq!(completed, instance_fixture::state(&pool).await);
}

async fn withdraw_source(pool: &PgPool, h: &Harness) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let mut tx = authz_postgres::begin_actor_transaction(&h.app, &h.actor)
        .await
        .unwrap();
    let allowed: i32 =
        sqlx::query_scalar("SELECT check_permission('user',$1,'can_use','release_agent',$2)")
            .bind(h.actor.user_id.to_string())
            .bind(h.fixture.release_agent.to_string())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(allowed, 0, "actual exact source authority was withdrawn");
}

async fn assert_denied(h: &Harness) {
    assert!(matches!(
        h.importer()
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await,
        Err(InstanceImportError::Deployment(
            DeploymentError::AuthorizationDenied
        ))
    ));
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn frozen_platform_policy_and_version_drift_hold_without_import_or_claim(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    h.prepared().await;
    let before = instance_fixture::state(&pool).await;
    for (memory, version) in [(256, "platform/import-v1"), (512, "platform/import-v2")] {
        let changed = Arc::new(PostgresExecutionPreparationRepository::new(
            h.app.clone(),
            instance_fixture::platform(memory, version),
            DeploymentExecutionProfile::RuntimeNamedV1,
        ));
        assert!(matches!(
            h.configured(changed, h.worker.clone())
                .import_prepared_instance(
                    &h.actor,
                    h.command,
                    h.intent.id(),
                    &fixtures::key("sqlite")
                )
                .await,
            Err(InstanceImportError::Deployment(
                DeploymentError::InputConflict
            ))
        ));
        assert_eq!(before, instance_fixture::state(&pool).await);
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn suspended_or_disabled_original_actor_cannot_execute_prepared_import(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    h.prepared().await;
    for status in ["suspended", "disabled"] {
        sqlx::query("UPDATE users SET status=$2 WHERE id=$1")
            .bind(h.actor.user_id.as_uuid())
            .bind(status)
            .execute(&pool)
            .await
            .unwrap();
        let before = instance_fixture::state(&pool).await;
        assert_denied(&h).await;
        assert_eq!(before, instance_fixture::state(&pool).await);
    }
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn foreign_actor_wrong_logical_command_and_volume_resource_do_not_import(pool: PgPool) {
    let h = Harness::new(&pool, true).await;
    h.prepared().await;
    let before = instance_fixture::state(&pool).await;
    let outsider = support::identity(h.fixture.outsider);
    let mut wrong = h.actor.clone();
    wrong.idempotency_id = RequestId::new();
    for actor in [&outsider, &wrong] {
        assert!(matches!(
            h.importer()
                .import_prepared_instance(actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
                .await,
            Err(InstanceImportError::Deployment(
                DeploymentError::InputConflict
            ))
        ));
    }
    assert!(matches!(
        h.importer()
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("data"))
            .await,
        Err(InstanceImportError::Deployment(
            DeploymentError::InvalidAction
        ))
    ));
    assert_eq!(before, instance_fixture::state(&pool).await);
}
