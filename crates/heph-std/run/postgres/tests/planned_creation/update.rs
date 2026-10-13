use std::sync::Arc;

use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::ReleaseService;
use run_domain::RunCleanupVmTargetKind;
use run_orchestrator::{RunCleanupRepository, RunRepository};
use run_postgres::{PgRunCleanupRepository, PgRunRepository};
use sqlx::PgPool;
use vm_trait::VmId;

use super::{
    fixture::{Role, role_pool, scope},
    seed,
};

pub async fn exercise(pool: &PgPool) {
    let worker = role_pool(pool, Role::Worker).await;
    let service = ReleaseService::new(worker.clone(), Arc::new(PostgresMelangeAuthorizer));
    let (identity, hook) = seed::update(pool).await;
    service
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(pool, hook.hook_run_id).await;
    assert_prohibited(&worker, &command).await;
    actor_cannot_forge(pool, &identity, command.run_id).await;
    let repository = PgRunRepository::new(worker.clone()).with_vm_owner_scope(scope());
    let vm = VmId(command.run_id.to_string());
    let created = repository
        .create_run_with_vm_plan(&command, &scope(), &vm)
        .await
        .unwrap();
    assert!(created.created);
    assert_eq!(created.run.vm_id.as_deref(), Some(vm.0.as_str()));
    assert!(
        !repository
            .create_run_with_vm_plan(&command, &scope(), &vm)
            .await
            .unwrap()
            .created
    );
    assert!(
        sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now() WHERE run_id=$1")
            .bind(command.run_id.as_uuid())
            .execute(&worker)
            .await
            .is_err()
    );
    closed_before_consumption(pool, &worker, &service).await;
    worker.close().await;
}

async fn actor_cannot_forge(
    pool: &PgPool,
    identity: &identity_domain::AuthenticatedIdentity,
    run: runtime_types::RunId,
) {
    let actor = role_pool(pool, Role::Actor).await;
    let mut tx = authz_postgres::begin_actor_transaction(&actor, identity)
        .await
        .unwrap();
    let error = sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,command_id,state,requires_state,created_at,updated_at,vm_plan_admission_required) SELECT gen_random_uuid(),instance_id,instance_revision_id,release_id,release_agent_id,'update',gen_random_uuid(),'queued',requires_state,now(),now(),true FROM runs WHERE id=$1")
        .bind(run.as_uuid()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("42501")
    );
    tx.rollback().await.unwrap();
    let mut tx = authz_postgres::begin_actor_transaction(&actor, identity)
        .await
        .unwrap();
    assert!(sqlx::query("UPDATE run_vm_plan_admissions SET consumed_at=now(),consumption_kind='provision',vm_id='forged',vm_provider_namespace='cleanup-root-owner-1',vm_host_id='cleanup-host-1' WHERE run_id=$1")
        .bind(run.as_uuid()).execute(&mut *tx).await.is_err());
    tx.rollback().await.unwrap();
    actor.close().await;
}

async fn assert_prohibited(worker: &PgPool, command: &run_domain::StartRun) {
    assert!(
        sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .execute(worker)
            .await
            .is_err()
    );
    assert!(sqlx::query("UPDATE runs SET vm_id='forged',vm_provider_namespace='cleanup-root-owner-1',vm_host_id='cleanup-host-1',vm_binding_kind='planned' WHERE id=$1")
        .bind(command.run_id.as_uuid()).execute(worker).await.is_err());
    assert!(
        sqlx::query("UPDATE runs SET vm_plan_admission_required=false WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .execute(worker)
            .await
            .is_err()
    );
}

async fn closed_before_consumption(pool: &PgPool, worker: &PgPool, service: &ReleaseService) {
    let (identity, hook) = seed::update(pool).await;
    service
        .begin_update_hook(&identity, hook.clone())
        .await
        .unwrap();
    let command = seed::load_hook(pool, hook.hook_run_id).await;
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=now() WHERE id=$1")
        .bind(command.run_id.as_uuid())
        .execute(worker)
        .await
        .unwrap();
    sqlx::query("DELETE FROM project_maintainers WHERE user_id=$1")
        .bind(identity.user_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("UPDATE releases SET state='revoked',revoked_at=now() WHERE id=$1")
        .bind(command.release_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    let repository = PgRunRepository::new(worker.clone()).with_vm_owner_scope(scope());
    assert!(
        repository
            .create_run_with_vm_plan(&command, &scope(), &VmId(command.run_id.to_string()))
            .await
            .is_err()
    );
    let cleanup = PgRunCleanupRepository::new(worker.clone(), scope());
    let target = cleanup.begin_cleanup(command.run_id).await.unwrap();
    assert!(target.leases().is_empty());
    assert_eq!(target.vm_target().kind(), RunCleanupVmTargetKind::Planned);
    assert_eq!(
        target.vm_target().vm_id(),
        Some(&VmId(command.run_id.to_string()))
    );
    assert_eq!(cleanup.begin_cleanup(command.run_id).await.unwrap(), target);
    let kind: String =
        sqlx::query_scalar("SELECT consumption_kind FROM run_vm_plan_admissions WHERE run_id=$1")
            .bind(command.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(kind, "cleanup");
    assert_prohibited(worker, &command).await;
}
