use authz_domain::{ObjectRef, ObjectType, Permission};
use identity_domain::RequestId;
use release_domain::AgentInstanceRevisionId;
use release_service::{InstanceExecutionError, InstanceExecutionService, InvokeInstance};
use runtime_types::RunId;
use sqlx::PgPool;
use uuid::Uuid;

use super::fixture::{Invocation, with_run};

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn source_use_withdrawal_denies_first_and_replay_without_graph_changes(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    withdraw(
        &pool,
        h.activation.fixture.project,
        h.activation.actor.user_id.as_uuid(),
    )
    .await;
    assert!(!h.activation.can_use(&h.activation.actor).await);
    let before = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, before);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.activation.fixture.project)
        .bind(h.activation.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    withdraw(
        &pool,
        h.activation.fixture.project,
        h.activation.actor.user_id.as_uuid(),
    )
    .await;
    let mut retry = h.activation.actor.clone();
    retry.request_id = RequestId::new();
    assert!(!h.activation.can_use(&retry).await);
    let committed = h.rows().await;
    assert!(matches!(
        h.service().invoke_instance(&retry, h.command).await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, committed);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn current_execute_and_active_user_are_required_before_replay(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    withdraw(
        &pool,
        h.activation.fixture.consuming_project,
        h.activation.actor.user_id.as_uuid(),
    )
    .await;
    assert!(
        !h.activation
            .permission(
                &h.activation.actor,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentInstance, h.command.instance_id().as_uuid())
            )
            .await
    );
    let before = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, before);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$2)")
        .bind(h.activation.fixture.consuming_project)
        .bind(h.activation.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(h.activation.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let suspended = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, suspended);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn different_current_caller_can_invoke_after_activation_issuer_loss(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    sqlx::query("UPDATE users SET status='suspended' WHERE id=$1")
        .bind(h.activation.actor.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let caller = crate::support::identity(h.activation.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$3),($2,$3)")
        .bind(h.activation.fixture.project)
        .bind(h.activation.fixture.consuming_project)
        .bind(caller.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    assert!(h.activation.can_use(&caller).await);
    assert!(
        h.activation
            .permission(
                &caller,
                Permission::CanExecute,
                ObjectRef::new(ObjectType::AgentInstance, h.command.instance_id().as_uuid())
            )
            .await
    );
    h.service()
        .invoke_instance(&caller, h.command)
        .await
        .unwrap();
    let lineage:bool=sqlx::query_scalar("SELECT q.actor_id=$2 AND a.actor_id=$3 AND p.actor_id=$3 AND q.actor_id<>a.actor_id FROM agent_instance_execution_invocations q JOIN agent_instance_execution_activations a ON a.id=q.activation_id JOIN recipe_deployment_execution_preparations p ON p.deployment_id=q.deployment_id WHERE q.id=$1")
        .bind(h.command.invocation_id().as_uuid()).bind(caller.user_id.as_uuid()).bind(h.activation.actor.user_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(
        lineage,
        "historical actor remains data, never a reconstructed authenticated identity"
    );
    assert_removed_history(&pool, &h, &caller).await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn changed_actor_and_input_cannot_borrow_original_invocation(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    let caller = crate::support::identity(h.activation.fixture.outsider);
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) VALUES($1,$3),($2,$3)")
        .bind(h.activation.fixture.project)
        .bind(h.activation.fixture.consuming_project)
        .bind(caller.user_id.as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let before = h.rows().await;
    assert!(matches!(
        h.service().invoke_instance(&caller, h.command).await,
        Err(InstanceExecutionError::InputConflict)
    ));
    let changed = with_run(h.command, RunId::from_uuid(Uuid::new_v4()));
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, changed)
            .await,
        Err(InstanceExecutionError::InputConflict)
    ));
    assert_eq!(h.rows().await, before);
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable PostgreSQL and private115"]
async fn changed_revision_and_closed_gate_never_retarget_or_reopen(pool: PgPool) {
    let h = Invocation::new(&pool).await;
    let changed = InvokeInstance::new(
        h.command.command_key(),
        h.command.invocation_id(),
        h.command.run_id(),
        h.command.start_command_id(),
        h.command.instance_id(),
        AgentInstanceRevisionId::from_uuid(Uuid::new_v4()),
    )
    .unwrap();
    let before = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, changed)
            .await,
        Err(InstanceExecutionError::StaleInstance)
    ));
    assert_eq!(h.rows().await, before);
    h.service()
        .invoke_instance(&h.activation.actor, h.command)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_instances SET run_gate_open=false,version=version+1 WHERE id=$1")
        .bind(h.command.instance_id().as_uuid())
        .execute(&pool)
        .await
        .unwrap();
    let closed = h.rows().await;
    assert!(matches!(
        h.service()
            .invoke_instance(&h.activation.actor, h.command)
            .await,
        Err(InstanceExecutionError::Closed)
    ));
    assert_eq!(h.rows().await, closed);
}

async fn withdraw(pool: &PgPool, project: Uuid, user: Uuid) {
    sqlx::query("DELETE FROM project_maintainers WHERE project_id=$1 AND user_id=$2")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .unwrap();
}

async fn assert_removed_history(
    pool: &PgPool,
    h: &Invocation,
    caller: &identity_domain::AuthenticatedIdentity,
) {
    let snapshot = h
        .activation
        .recipe
        .inspect(caller, h.activation.intent.id())
        .await
        .unwrap();
    h.activation
        .recipe
        .admit_remove(
            caller,
            crate::fixtures::remove(caller, h.activation.intent.id(), snapshot.version),
        )
        .await
        .unwrap();
    let immutable:bool=sqlx::query_scalar("SELECT i.version=2 AND i.run_gate_open AND recipe_cleanup_started(q.deployment_id) AND run_profile_producer_matches(r) AND qualified_invocation_graph_matches(q) AND NOT qualified_invocation_current_eligible(q.instance_id,q.revision_id,q.config_json) FROM agent_instance_execution_invocations q JOIN runs r ON r.id=q.run_id JOIN agent_instances i ON i.id=q.instance_id WHERE q.id=$1")
        .bind(h.command.invocation_id().as_uuid()).fetch_one(pool).await.unwrap();
    assert!(
        immutable,
        "admitted Remove blocks effects independently of still-open instance gate; immutable producer remains"
    );
    let fresh = InvokeInstance::new(
        release_domain::ReleaseCommandKey::derive(
            "after_remove",
            &[h.command.instance_id().as_uuid().as_bytes()],
        ),
        release_service::InstanceInvocationId::from_uuid(Uuid::new_v4()).unwrap(),
        RunId::from_uuid(Uuid::new_v4()),
        runtime_types::CommandId::from_uuid(Uuid::new_v4()),
        h.command.instance_id(),
        h.command.expected_revision_id(),
    )
    .unwrap();
    let removed = h.rows().await;
    for command in [h.command, fresh] {
        assert!(matches!(
            h.service().invoke_instance(caller, command).await,
            Err(InstanceExecutionError::Closed)
        ));
    }
    assert_eq!(h.rows().await, removed);
    withdraw(pool, h.activation.fixture.project, caller.user_id.as_uuid()).await;
    assert!(!h.activation.can_use(caller).await);
    let immutable: bool =
        sqlx::query_scalar("SELECT run_profile_producer_matches(r) FROM runs r WHERE id=$1")
            .bind(h.command.run_id().as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert!(
        immutable,
        "current Source withdrawal cannot erase immutable producer data or grant cleanup rights"
    );
    let withdrawn = h.rows().await;
    assert!(matches!(
        h.service().invoke_instance(caller, h.command).await,
        Err(InstanceExecutionError::AuthorizationDenied)
    ));
    assert_eq!(h.rows().await, withdrawn);
}
