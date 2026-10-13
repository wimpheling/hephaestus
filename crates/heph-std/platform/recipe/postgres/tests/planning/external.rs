use recipe_application::DeploymentError;
use sqlx::PgPool;
use uuid::Uuid;

use crate::{fixture, fixtures, support};

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn unselected_external_observation_requires_only_read(pool: PgPool) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let actor = support::identity(seeded.maintainer);
    let source = fixtures::source(&seeded, true);
    let source = source
        .split("[[resources]]\nkind = \"instance\"")
        .next()
        .unwrap();
    let request = recipe_application::PlanningRequest::new(
        forge_domain::ProjectId::from_uuid(seeded.consuming_project),
        recipe_application::DeploymentKey::parse("external-reference").unwrap(),
        source.as_bytes(),
        std::collections::BTreeMap::new(),
        std::collections::BTreeMap::from([(
            fixtures::key("data"),
            runtime_types::VolumeId::from_uuid(seeded.volume),
        )]),
    )
    .unwrap();
    fixture::planner(&pool, 512)
        .await
        .plan(&actor, &request)
        .await
        .unwrap();
    let permissions: Vec<String> = sqlx::query_scalar("SELECT permission FROM authorization_audit_events WHERE request_id=$1 AND object_type='state_volume' AND object_id=$2 ORDER BY permission")
        .bind(actor.request_id.as_uuid()).bind(seeded.volume).fetch_all(&pool).await.unwrap();
    assert_eq!(permissions, ["can_read"]);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn selected_ready_external_checks_read_attach_and_grant_without_creating_grants(
    pool: PgPool,
) {
    let seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let actor = support::identity(seeded.maintainer);
    let plan = fixture::planner(&pool, 512)
        .await
        .plan(&actor, &fixture::request(&seeded, true))
        .await
        .unwrap();
    assert_eq!(
        plan.intent().resources()[&fixtures::key("data")].ownership(),
        recipe_application::ResourceOwnership::External
    );
    let permissions: Vec<String> = sqlx::query_scalar("SELECT permission FROM authorization_audit_events WHERE request_id=$1 AND object_type='state_volume' AND object_id=$2 ORDER BY permission")
        .bind(actor.request_id.as_uuid()).bind(seeded.volume).fetch_all(&pool).await.unwrap();
    assert_eq!(
        permissions,
        ["can_attach", "can_grant_agent_capability", "can_read"]
    );
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_mount_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grants, 0);
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn unready_external_cannot_be_observed_as_ready(pool: PgPool) {
    let mut seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let volume = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,capacity_bytes,state,filesystem_uuid) VALUES($1,$2,16777216,'uninitialized',$3)")
        .bind(volume).bind(seeded.consuming_project).bind(Uuid::new_v4()).execute(&pool).await.unwrap();
    seeded.volume = volume;
    assert!(matches!(
        fixture::planner(&pool, 512)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, true)
            )
            .await,
        Err(DeploymentError::Unavailable)
    ));
}

#[sqlx::test(migrations = "../../../../../migrations")]
#[ignore = "requires disposable PostgreSQL through DATABASE_URL"]
async fn external_same_project_is_required_even_when_actor_manages_both_projects(pool: PgPool) {
    let mut seeded = fixture::setup(&pool, fixture::contract(), true, true).await;
    let volume = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,capacity_bytes,state,filesystem_uuid,host_id,host_path,provisioning_state)
        SELECT $1,$2,capacity_bytes,'ready',$3,host_id,'/tmp/planner-wrong-project.raw','ready'
        FROM agent_instance_state_volumes WHERE id=$4")
        .bind(volume).bind(seeded.project).bind(Uuid::new_v4()).bind(seeded.volume)
        .execute(&pool).await.unwrap();
    seeded.volume = volume;
    assert!(matches!(
        fixture::planner(&pool, 512)
            .await
            .plan(
                &support::identity(seeded.maintainer),
                &fixture::request(&seeded, true)
            )
            .await,
        Err(DeploymentError::Unavailable)
    ));
}
