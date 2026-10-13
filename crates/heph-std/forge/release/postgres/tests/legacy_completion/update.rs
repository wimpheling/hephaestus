#[path = "retry.rs"]
mod retry;

use super::fixture;
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use release_domain::{AgentUpdateId, ReleaseCommandKey};
use release_service::BeginUpdateHook;
use run_domain::{RunKind, StartRun};
use runtime_types::{AgentInstanceRevisionId, RunId};
use sqlx::PgPool;
use uuid::Uuid;

async fn pending(pool: &PgPool, run: &StartRun) -> (AuthenticatedIdentity, BeginUpdateHook) {
    let candidate = AgentInstanceRevisionId::new();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable FROM agent_instance_revisions WHERE id=$2").bind(candidate.as_uuid()).bind(run.instance_revision_id.as_uuid()).execute(pool).await.unwrap();
    let actor = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Release callback fixture')")
        .bind(actor)
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO project_maintainers(project_id,user_id) SELECT project_id,$2 FROM agent_instances WHERE id=$1").bind(run.instance_id.as_uuid()).bind(actor).execute(pool).await.unwrap();
    let update = AgentUpdateId::new();
    sqlx::query("INSERT INTO agent_updates(id,instance_id,expected_current_revision_id,candidate_revision_id,state,actor_id) VALUES($1,$2,$3,$4,'draining',$5)")
        .bind(update.as_uuid()).bind(run.instance_id.as_uuid()).bind(run.instance_revision_id.as_uuid()).bind(candidate.as_uuid()).bind(actor).execute(pool).await.unwrap();
    sqlx::query("UPDATE agent_instances SET state='update_draining',run_gate_open=false,version=version+1 WHERE id=$1").bind(run.instance_id.as_uuid()).execute(pool).await.unwrap();
    (
        AuthenticatedIdentity::new(
            UserId::from_uuid(actor),
            "fixture",
            actor.to_string(),
            serde_json::json!({}),
            RequestId::new(),
        ),
        BeginUpdateHook {
            command_key: ReleaseCommandKey::derive("observer-hook", &[update.as_uuid().as_bytes()]),
            update_id: update,
            hook_run_id: RunId::new(),
        },
    )
}

async fn hook(
    pool: &PgPool,
    worker: &PgPool,
    scope: &run_domain::LegacyVmPlacementScope,
) -> (BeginUpdateHook, StartRun) {
    let (actor, command) = super::seed::update(pool).await;
    fixture::service(worker, scope)
        .begin_update_hook(&actor, command.clone())
        .await
        .unwrap();
    let run = super::seed::load_hook(pool, command.hook_run_id).await;
    fixture::consume(worker, scope, &run).await;
    (command, run)
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn deferred_hook_birth_requires_scoped_normal_cleanup_not_bare_terminal_state(pool: PgPool) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let normal = fixture::fresh(&pool, &worker, &scope, false, false).await;
    fixture::running(&worker, &normal).await;
    let (actor, command) = pending(&pool, &normal).await;
    let before = fixture::snapshot(&pool, &normal).await;
    assert!(
        fixture::service(&worker, &scope)
            .begin_update_hook(&actor, command.clone())
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &normal).await, before);
    fixture::finish(&worker, &scope, &normal).await;
    fixture::service(&worker, &scope)
        .begin_update_hook(&actor, command.clone())
        .await
        .unwrap();
    let qualified:bool=sqlx::query_scalar("SELECT execution.state='queued' AND execution.legacy_vm_placement_required AND plan.consumed_at IS NULL FROM runs execution JOIN run_legacy_vm_placements plan ON plan.run_id=execution.id WHERE execution.id=$1")
        .bind(command.hook_run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(qualified);
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn foreign_terminal_normal_drain_does_not_create_hook_or_mutate_update(pool: PgPool) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let normal = fixture::fresh(&pool, &worker, &scope, false, false).await;
    fixture::running(&worker, &normal).await;
    fixture::finish(&worker, &scope, &normal).await;
    let (actor, command) = pending(&pool, &normal).await;
    let before = fixture::snapshot(&pool, &normal).await;
    assert!(
        fixture::service(&worker, &fixture::owner())
            .begin_update_hook(&actor, command.clone())
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &normal).await, before);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM runs WHERE id=$1")
        .bind(command.hook_run_id.as_uuid())
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0);
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn committed_update_only_activates_after_qualified_cleanup_and_replays_without_reset(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let (command, run) = hook(&pool, &worker, &scope).await;
    fixture::running(&worker, &run).await;
    fixture::close(&worker, &scope, &run).await;
    fixture::outcome(&worker, &run, true).await;
    let before = fixture::snapshot(&pool, &run).await;
    assert!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &run).await, before);
    fixture::transition(&worker, &run, "cleaned_up").await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .legacy_run_cleanup_completed(run.run_id)
            .await
            .unwrap(),
        RunKind::Update
    );
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        release_service::UpdateDecision::Activated
    );
    let after = fixture::snapshot(&pool, &run).await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        release_service::UpdateDecision::Activated
    );
    assert_eq!(fixture::snapshot(&pool, &run).await, after);
    assert_eq!(after["updates"][0]["id"], command.update_id.to_string());
    worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires disposable private Legacy completion PostgreSQL"]
async fn rejected_update_preserves_original_producer_and_does_not_require_canonical_receipt(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    let scope = fixture::owner();
    let worker = fixture::role(&pool, true).await;
    let normal = fixture::fresh(&pool, &worker, &scope, true, false).await;
    fixture::close(&worker, &scope, &normal).await;
    fixture::outcome(&worker, &normal, false).await;
    fixture::transition(&worker, &normal, "cleaned_up").await;
    let (actor, command) = pending(&pool, &normal).await;
    fixture::service(&worker, &scope)
        .begin_update_hook(&actor, command.clone())
        .await
        .unwrap();
    let run = super::seed::load_hook(&pool, command.hook_run_id).await;
    fixture::consume(&worker, &scope, &run).await;
    let released = fixture::acquire(&pool, &worker, &scope, &run, None).await;
    fixture::running(&worker, &run).await;
    fixture::close(&worker, &scope, &run).await;
    fixture::outcome(&worker, &run, false).await;
    fixture::detach(&worker, &scope, &run, &released).await;
    fixture::transition(&worker, &run, "cleaned_up").await;
    let replacement = Uuid::new_v4();
    sqlx::query("INSERT INTO users(id,display_name) VALUES($1,'Replacement fixture actor')")
        .bind(replacement)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_updates SET actor_id=$2 WHERE id=$1")
        .bind(command.update_id.as_uuid())
        .bind(replacement)
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        release_service::UpdateDecision::AgentRejected
    );
    let evidence:bool=sqlx::query_scalar("SELECT plan.actor_id<>update.actor_id AND execution.state='cleaned_up' AND NOT EXISTS(SELECT 1 FROM run_cleanup_receipts WHERE run_id=execution.id) FROM run_legacy_vm_placements plan JOIN runs execution ON execution.id=plan.run_id JOIN agent_updates update ON update.id=plan.producer_id WHERE execution.id=$1")
        .bind(run.run_id.as_uuid()).fetch_one(&pool).await.unwrap();
    assert!(evidence);
    let next = super::support::retry_command(&normal);
    fixture::birth(&pool, &worker, &scope, &next, false).await;
    let held = fixture::acquire(&pool, &worker, &scope, &next, Some(released.volume_id)).await;
    assert!(held.fencing_token > released.fencing_token);
    let before = fixture::snapshot(&pool, &run).await;
    assert_eq!(
        fixture::service(&worker, &scope)
            .reconcile_update_run(run.run_id)
            .await
            .unwrap(),
        release_service::UpdateDecision::AgentRejected
    );
    assert!(
        fixture::service(&worker, &fixture::owner())
            .reconcile_update_run(run.run_id)
            .await
            .is_err()
    );
    assert_eq!(fixture::snapshot(&pool, &run).await, before);
    worker.close().await;
}
