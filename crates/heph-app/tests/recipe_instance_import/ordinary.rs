use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use hephaestus_app::recipe_instance_import::to_import_command;
use release_domain::{AgentInstanceId, AgentInstanceRevisionId, InstanceName, ReleaseCommandKey};
use release_service::ImportAgent;
use sqlx::PgPool;
use uuid::Uuid;

use crate::instance_fixture::Harness;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private113 migrations"]
async fn ordinary107_legacy_import_keeps_original_mode_and_receipt_without_recipe_adoption(
    pool: PgPool,
) {
    let h = Harness::new(&pool, false).await;
    let typed = to_import_command(&h.prospective_slotless_import());
    let id = AgentInstanceId::from_uuid(Uuid::new_v4());
    let revision = AgentInstanceRevisionId::from_uuid(Uuid::new_v4());
    let key = ReleaseCommandKey::derive("ordinary-legacy", &[id.as_uuid().as_bytes()]);
    // Frozen107 legacy inbox uses the existing worker service path; its
    // explicit authorizer still receives the SAME verified current user.
    // This does not claim legacy actor-role inbox INSERT is supported.
    let historical = release_postgres::ReleaseService::new(
        h.worker.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let actual = historical
        .import_agent(
            &h.actor,
            ImportAgent {
                command_key: key,
                instance_id: id,
                revision_id: revision,
                project_id: typed.project_id,
                release_agent_id: typed.release_agent_id,
                name: InstanceName::parse("ordinary-legacy").unwrap(),
                parameters: typed.parameters,
                selected_policy: typed.selected_policy,
                platform_policy: typed.platform_policy,
                platform_policy_version: typed.platform_policy_version,
            },
        )
        .await
        .unwrap();
    assert_eq!(actual, id);
    let facts:(String,Uuid,String,i64,i64)=sqlx::query_as("SELECT instance.volume_mode,instance.active_revision_id,inbox.operation,(SELECT count(*) FROM recipe_prepared_instance_imports),(SELECT count(*) FROM recipe_effect_attempts) FROM agent_instances instance JOIN release_command_inbox inbox ON inbox.aggregate_id=instance.id WHERE instance.id=$1 AND inbox.command_key=$2")
        .bind(id.as_uuid()).bind(key.as_bytes().as_slice()).fetch_one(&pool).await.unwrap();
    assert_eq!(
        facts,
        (
            "legacy".to_owned(),
            revision.as_uuid(),
            "import_agent".to_owned(),
            0,
            0
        )
    );
}
