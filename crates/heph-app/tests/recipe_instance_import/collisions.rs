use hephaestus_app::recipe_instance_import::to_import_command;
use recipe_application::DeploymentExecutionPreparationRepository;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{
    fixtures,
    instance_fixture::{self, Harness},
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn independent_matching_predicted_instance_is_not_adopted_as_recipe_creation(pool: PgPool) {
    let h = Harness::new(&pool, false).await;
    let prospective = h.prospective_slotless_import();
    let actual = h
        .release
        .import_agent_with_volumes(&h.actor, to_import_command(&prospective))
        .await
        .unwrap();
    assert_eq!(actual.as_uuid(), prospective.instance_id().as_uuid());
    assert_unadopted(&pool, &h).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn independently_committed_same_command_key_with_foreign_instance_blocks_fresh_preparation(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prospective = h.prospective_slotless_import();
    let mut command = to_import_command(&prospective);
    command.instance_id = release_domain::AgentInstanceId::from_uuid(Uuid::new_v4());
    command.revision_id = release_domain::AgentInstanceRevisionId::from_uuid(Uuid::new_v4());
    let foreign = command.instance_id;
    let actual = h
        .release
        .import_agent_with_volumes(&h.actor, command)
        .await
        .unwrap();
    assert_eq!(actual, foreign);
    assert_ne!(actual.as_uuid(), prospective.instance_id().as_uuid());
    assert_unadopted(&pool, &h).await;
}

async fn assert_unadopted(pool: &PgPool, h: &Harness) {
    let before = instance_fixture::state(pool).await;
    assert!(
        h.importer()
            .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
            .await
            .is_err()
    );
    assert_eq!(before, instance_fixture::state(pool).await);
    assert!(
        h.preparation
            .inspect_execution_preparation(&h.actor, h.intent.id())
            .await
            .unwrap()
            .is_none()
    );
    let counts:(i64,i64,i64)=sqlx::query_as("SELECT (SELECT count(*) FROM recipe_effect_attempts),
      (SELECT count(*) FROM recipe_effect_verifications),(SELECT count(*) FROM recipe_execution_transitions)")
      .fetch_one(pool).await.unwrap();
    assert_eq!(
        counts,
        (0, 0, 0),
        "no original claim, verified creation or ownership adoption was manufactured"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn generic_service_cannot_commit_changed_prepared_policy_version_under_reserved_key(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let prepared = h.prepared().await;
    let claim = h.claim().await;
    let before = instance_fixture::state(&pool).await;
    let mut changed = to_import_command(&prepared);
    changed.platform_policy_version = "different-platform/version".to_owned();
    assert!(
        h.release
            .import_agent_with_volumes(&h.actor, changed)
            .await
            .is_err()
    );
    assert_eq!(before, instance_fixture::state(&pool).await);
    assert_eq!(h.active().await, Some(claim));
    h.importer()
        .import_prepared_instance(&h.actor, h.command, h.intent.id(), &fixtures::key("sqlite"))
        .await
        .unwrap();
}
