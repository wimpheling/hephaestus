use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::RequestId;
use release_service::{InstanceExecutionError, InstanceExecutionService};
use sqlx::PgPool;

use super::fixture::Harness;

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn live_manage_and_source_use_are_checked_before_first_and_replay(pool: PgPool) {
    let h = Harness::new(&pool).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(!h.can_use(&h.actor).await);
    let before = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, before);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    h.service()
        .activate_instance(&h.actor, h.command)
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.consuming_project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut retry = h.actor.clone();
    retry.request_id = RequestId::new();
    assert!(
        !h.permission(
            &retry,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, h.command.instance_id().as_uuid())
        )
        .await
    );
    let after = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&retry, h.command).await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, after);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private114"]
async fn active_different_manager_can_activate_after_original_creator_loss(pool: PgPool) {
    let h = Harness::new(&pool).await;
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(matches!(
        h.service().activate_instance(&h.actor, h.command).await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, before);
    let manager = crate::support::identity(h.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$3),($2,$3)")
        .bind(h.fixture.project)
        .bind(h.fixture.consuming_project)
        .bind(manager.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(h.can_use(&manager).await);
    assert!(
        h.permission(
            &manager,
            Permission::CanManage,
            ObjectRef::new(ObjectType::AgentInstance, h.command.instance_id().as_uuid())
        )
        .await
    );
    let first = h
        .service()
        .activate_instance(&manager, h.command)
        .await
        .unwrap();
    let after = h.rows().await;
    let mut retry = manager;
    retry.request_id = RequestId::new();
    assert_eq!(
        h.service()
            .activate_instance(&retry, h.command)
            .await
            .unwrap(),
        first
    );
    assert_eq!(h.rows().await, after);
    let lineage:bool=sqlx::query_scalar("SELECT a.actor_id=$2 AND p.actor_id=$3 FROM agent_instance_execution_activations a JOIN recipe_deployment_execution_preparations p ON p.deployment_id=a.deployment_id WHERE a.instance_id=$1")
        .bind(h.command.instance_id().as_uuid()).bind(retry.user_id.as_uuid()).bind(h.actor.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        lineage,
        "current activation manager and immutable original actor are distinct"
    );
}
