use run_domain::{RunCleanupVmTargetKind, StartRun};
use run_orchestrator::{RunCleanupRepository, RunRepository};
use run_postgres::{PgRunCleanupRepository, PgRunRepository};
use sqlx::PgPool;
use vm_trait::VmId;

use super::{
    fixture::{Role, role_pool, scope},
    seed,
};

pub async fn exercise(pool: &PgPool, historical: &StartRun) {
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunRepository::new(worker.clone()).with_vm_owner_scope(scope());
    assert!(
        repository
            .create_run_with_vm_plan(historical, &scope(), &VmId(historical.run_id.to_string()))
            .await
            .is_err()
    );
    rollback_cases(pool, &repository).await;
    let command = seed::normal(pool).await;
    let vm = VmId(command.run_id.to_string());
    assert!(
        repository
            .create_run_with_vm_plan(&command, &scope(), &VmId("another-run-vm".into()))
            .await
            .is_err()
    );
    let absent: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM runs WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=$2)")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert!(absent);
    let first = repository
        .create_run_with_vm_plan(&command, &scope(), &vm)
        .await
        .unwrap();
    assert!(first.created);
    assert_eq!(first.run.vm_id.as_deref(), Some(vm.0.as_str()));
    assert!(
        !repository
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .unwrap()
            .created
    );
    immutable_replay(&worker, &repository, &command, &vm).await;
    let cleanup = PgRunCleanupRepository::new(worker.clone(), scope());
    // Simulates process loss immediately after atomic creation committed.
    let target = cleanup.begin_cleanup(command.run_id).await.unwrap();
    assert!(target.leases().is_empty());
    assert_eq!(target.vm_target().kind(), RunCleanupVmTargetKind::Planned);
    assert_eq!(target.vm_target().host(), Some(&scope()));
    assert_eq!(target.vm_target().vm_id(), Some(&vm));
    assert!(
        repository
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .is_err()
    );
    actor_denied(pool).await;
    worker.close().await;
}

async fn rollback_cases(pool: &PgPool, repository: &PgRunRepository) {
    let valid = seed::normal(pool).await;
    let mut wrong = valid.clone();
    wrong.requires_state = true;
    assert_rejected(pool, repository, &wrong, &scope()).await;
    wrong = valid.clone();
    wrong.release_id = runtime_types::ReleaseId::new();
    assert_rejected(pool, repository, &wrong, &scope()).await;
    wrong = valid.clone();
    wrong.release_agent_id = runtime_types::ReleaseAgentId::new();
    assert_rejected(pool, repository, &wrong, &scope()).await;
    wrong = valid.clone();
    wrong.instance_revision_id = runtime_types::AgentInstanceRevisionId::new();
    assert_rejected(pool, repository, &wrong, &scope()).await;
    let foreign =
        run_domain::RunCleanupHostId::new("foreign-owner".into(), "cleanup-host-1".into()).unwrap();
    assert_rejected(pool, repository, &valid, &foreign).await;
    sqlx::query("UPDATE agent_instances SET run_gate_open=false WHERE id=$1")
        .bind(valid.instance_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert_rejected(pool, repository, &valid, &scope()).await;
    let revoked = seed::normal(pool).await;
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(revoked.release_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    assert_rejected(pool, repository, &revoked, &scope()).await;
}

async fn assert_rejected(
    pool: &PgPool,
    repository: &PgRunRepository,
    command: &StartRun,
    owner: &run_domain::RunCleanupHostId,
) {
    assert!(
        repository
            .create_run_with_vm_plan(command, owner, &VmId(command.run_id.to_string()))
            .await
            .is_err()
    );
    let empty: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM runs WHERE id=$1) AND NOT EXISTS(SELECT 1 FROM command_inbox WHERE command_id=$2) AND NOT EXISTS(SELECT 1 FROM run_events WHERE run_id=$1)")
        .bind(command.run_id.as_uuid()).bind(command.command_id.as_uuid()).fetch_one(pool).await.unwrap();
    assert!(empty, "denied admission must roll back inbox/run/events");
}

async fn immutable_replay(
    worker: &PgPool,
    repository: &PgRunRepository,
    command: &StartRun,
    vm: &VmId,
) {
    let mut changed = command.clone();
    changed.requires_state = true;
    assert!(
        repository
            .create_run_with_vm_plan(&changed, &scope(), vm)
            .await
            .is_err()
    );
    assert!(
        repository
            .create_run_with_vm_plan(command, &scope(), &VmId("different-vm".into()))
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE command_inbox SET payload='{}' WHERE command_id=$1")
            .bind(command.command_id.as_uuid())
            .execute(worker)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE runs SET requires_state=NOT requires_state WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .execute(worker)
            .await
            .is_err()
    );
}

async fn actor_denied(pool: &PgPool) {
    let command = seed::normal(pool).await;
    let actor = role_pool(pool, Role::Actor).await;
    let repository = PgRunRepository::new(actor.clone()).with_vm_owner_scope(scope());
    assert_rejected(pool, &repository, &command, &scope()).await;
    actor.close().await;
}
