use identity_domain::{AuthenticatedIdentity, RequestId};
use recipe_application::{DeploymentError, DeploymentExecutionPreparationRepository};
use sqlx::PgPool;

use crate::{
    harness::{self, Graph, Harness},
    support,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn source_withdrawal_denies_replay_but_allows_historical_manager_inspection(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let first = h.prepare().await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut actor = h.actor.clone();
    actor.request_id = RequestId::new();
    assert!(
        !h.can_use(&actor).await,
        "exact live source CanUse was withdrawn"
    );
    assert!(matches!(
        h.preparation
            .prepare_execution(&actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&actor, h.intent.id())
            .await
            .unwrap(),
        Some(first.preparation.clone())
    );
    let manager = manager_without_source(&h, &pool).await;
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&manager, h.intent.id())
            .await
            .unwrap(),
        Some(first.preparation)
    );
    assert!(
        matches!(
            h.preparation
                .prepare_execution(&manager, h.command, h.intent.id())
                .await,
            Err(DeploymentError::InputConflict)
        ),
        "manager cannot borrow original actor command"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn project_authority_is_fresh_before_replay_and_inspection(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    h.prepare().await;
    let before = harness::evidence(&pool).await;
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.consuming_project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut actor = h.actor.clone();
    actor.request_id = RequestId::new();
    assert!(matches!(
        h.preparation
            .prepare_execution(&actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert!(matches!(
        h.preparation
            .inspect_execution_preparation(&actor, h.intent.id())
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(before, harness::evidence(&pool).await);
    let denies: i64 = sqlx::query_scalar("SELECT count(*) FROM authorization_audit_events WHERE request_id=$1 AND permission='can_manage' AND decision='deny'")
        .bind(actor.request_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        denies >= 2,
        "both denials remain durable outside rolled back operation"
    );
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn bound_external_records_read_attach_grant_without_minting_any_grant(pool: PgPool) {
    let h = Harness::new(&pool, Graph::External, 113).await;
    h.prepare().await;
    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT permission FROM recipe_execution_preparation_audits
        WHERE deployment_id=$1 AND object_type='state_volume' AND object_id=$2 ORDER BY permission",
    )
    .bind(h.intent.id().as_uuid())
    .bind(h.fixture.volume)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(
        permissions,
        ["can_attach", "can_grant_agent_capability", "can_read"]
    );
    let grants: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_mount_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(grants, 0);
    let before = harness::evidence(&pool).await;
    // The external volume belongs to this target project; its manager-derived
    // rights cannot be removed independently of ProjectCanManage in this model.
    // Withdraw that real relation and prove all three exact volume checks deny.
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(h.fixture.consuming_project)
        .bind(h.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let mut tx = authz_postgres::begin_actor_transaction(&h.app, &h.actor)
        .await
        .unwrap();
    let rights: (i32, i32, i32) = sqlx::query_as(
        "SELECT check_permission('user',$1,'can_read','state_volume',$2),
          check_permission('user',$1,'can_attach','state_volume',$2),
          check_permission('user',$1,'can_grant_agent_capability','state_volume',$2)",
    )
    .bind(h.actor.user_id.to_string())
    .bind(h.fixture.volume.to_string())
    .fetch_one(&mut *tx)
    .await
    .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(rights, (0, 0, 0));
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::AuthorizationDenied)
    ));
    assert_eq!(before, harness::evidence(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn unused_external_reference_records_read_only(pool: PgPool) {
    let h = Harness::new(&pool, Graph::ReferenceOnly, 113).await;
    let result = h.prepare().await;
    assert!(result.preparation.instances().is_empty());
    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT permission FROM recipe_execution_preparation_audits
        WHERE deployment_id=$1 AND object_type='state_volume' AND object_id=$2 ORDER BY permission",
    )
    .bind(h.intent.id().as_uuid())
    .bind(h.fixture.volume)
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(permissions, ["can_read"]);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn unready_external_cannot_be_frozen_from_coarse_admission_metadata(pool: PgPool) {
    let h = Harness::new(&pool, Graph::UnreadyExternal, 113).await;
    let before = harness::evidence(&pool).await;
    assert!(matches!(
        h.preparation
            .prepare_execution(&h.actor, h.command, h.intent.id())
            .await,
        Err(DeploymentError::Unavailable)
    ));
    assert_eq!(before, harness::evidence(&pool).await);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and explicit private migrations"]
async fn inactive_original_actor_cannot_prepare_or_replay_protected_writer(pool: PgPool) {
    let h = Harness::new(&pool, Graph::Owned, 113).await;
    let first = h.prepare().await;
    let before = harness::evidence(&pool).await;
    for status in ["suspended", "disabled"] {
        sqlx::query("UPDATE users SET status=$1 WHERE id=$2")
            .bind(status)
            .bind(h.actor.user_id.as_uuid())
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            h.preparation
                .prepare_execution(&h.actor, h.command, h.intent.id())
                .await,
            Err(DeploymentError::AuthorizationDenied)
        ));
        let mut tx = authz_postgres::begin_actor_transaction(&h.app, &h.actor)
            .await
            .unwrap();
        let children = first
            .preparation
            .instances()
            .iter()
            .map(|child| serde_json::to_vec(child).unwrap())
            .collect::<Vec<_>>();
        let error = sqlx::query("SELECT * FROM write_recipe_execution_preparation($1,$2,$3,$4,$5)")
            .bind(h.intent.id().as_uuid())
            .bind(h.command.id().as_uuid())
            .bind(first.preparation.canonical_bytes().unwrap())
            .bind(children)
            .bind(authz_postgres::AUTHORIZATION_MODEL_VERSION)
            .execute(&mut *tx)
            .await
            .unwrap_err();
        assert_eq!(
            error
                .as_database_error()
                .and_then(sqlx::error::DatabaseError::code)
                .as_deref(),
            Some("42501")
        );
        tx.rollback().await.unwrap();
    }
    let manager = manager_without_source(&h, &pool).await;
    assert_eq!(
        h.preparation
            .inspect_execution_preparation(&manager, h.intent.id())
            .await
            .unwrap(),
        Some(first.preparation)
    );
    assert_eq!(before, harness::evidence(&pool).await);
}

async fn manager_without_source(h: &Harness, pool: &PgPool) -> AuthenticatedIdentity {
    // Organization membership conveys source use; choose the seeded outsider
    // and grant only the actual target project's manager relation.
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.fixture.consuming_project)
        .bind(h.fixture.outsider.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let manager = support::identity(h.fixture.outsider);
    let mut tx = authz_postgres::begin_actor_transaction(&h.app, &manager)
        .await
        .unwrap();
    let project_allowed: bool =
        sqlx::query_scalar("SELECT check_permission('user',$1,'can_manage','project',$2)=1")
            .bind(manager.user_id.to_string())
            .bind(h.fixture.consuming_project.to_string())
            .fetch_one(&mut *tx)
            .await
            .unwrap();
    tx.rollback().await.unwrap();
    assert!(
        project_allowed,
        "different manager has live target ProjectCanManage"
    );
    assert!(
        !h.can_use(&manager).await,
        "different manager has no source CanUse"
    );
    manager
}
