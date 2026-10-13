use run_domain::{RunCleanupTarget, RunCleanupVmTarget};
use run_orchestrator::RunCleanupRepository;
use run_postgres::PgRunCleanupRepository;
use sqlx::PgPool;
use vm_trait::VmId;

use super::fixture::{Fixture, Role, role_pool, scope};

pub async fn foreign_host_stays_held(pool: &PgPool, fixture: &Fixture) {
    let repository = PgRunCleanupRepository::new(role_pool(pool, Role::Worker).await, scope());
    repository
        .bind_vm_before_provision(
            fixture.command.run_id,
            fixture.command.instance_id,
            fixture.command.instance_revision_id,
            &scope(),
            &VmId(fixture.command.run_id.to_string()),
        )
        .await
        .expect("known VM ownership");
    assert!(
        repository
            .begin_cleanup(fixture.command.run_id)
            .await
            .is_err()
    );
    let unchanged: bool = sqlx::query_scalar("SELECT volume_acquisition_closed_at IS NULL AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=$1) AND EXISTS(SELECT 1 FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL) FROM runs WHERE id=$1")
        .bind(fixture.command.run_id.as_uuid()).fetch_one(pool).await.expect("wrong host held atomically");
    assert!(
        unchanged,
        "known VM ownership cannot produce a receipt for a foreign lease host"
    );
}

pub async fn incomplete_snapshot_rolls_back(pool: &PgPool, fixture: &Fixture) {
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunCleanupRepository::new(worker.clone(), scope());
    let vm = VmId(fixture.command.run_id.to_string());
    repository
        .bind_vm_before_provision(
            fixture.command.run_id,
            fixture.command.instance_id,
            fixture.command.instance_revision_id,
            &scope(),
            &vm,
        )
        .await
        .expect("planned VM");
    let omitted = RunCleanupTarget::new(
        fixture.command.run_id,
        fixture.command.instance_id,
        fixture.command.instance_revision_id,
        RunCleanupVmTarget::planned(scope(), vm.clone()).expect("shape"),
        1,
        vec![],
    )
    .expect("missing lease candidate");
    let mut tx = worker
        .begin()
        .await
        .expect("raw worker snapshot transaction");
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(fixture.command.instance_id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("consumer first");
    sqlx::query("UPDATE runs SET volume_acquisition_closed_at=now() WHERE id=$1")
        .bind(fixture.command.run_id.as_uuid())
        .execute(&mut *tx)
        .await
        .expect("close run lock");
    sqlx::query("INSERT INTO run_cleanup_targets(run_id,instance_id,instance_revision_id,generation,vm_target_kind,vm_provider_namespace,vm_host_id,vm_id,target_digest,lease_count) VALUES($1,$2,$3,1,'planned','cleanup-root-owner-1','cleanup-host-1',$4,$5,0)")
        .bind(fixture.command.run_id.as_uuid()).bind(fixture.command.instance_id.as_uuid())
        .bind(fixture.command.instance_revision_id.as_uuid()).bind(vm.0).bind(omitted.digest().as_slice())
        .execute(&mut *tx).await.expect("candidate missing canonical fence");
    assert!(
        tx.commit().await.is_err(),
        "deferred snapshot guard must reject an omitted lease"
    );
    let open: bool = sqlx::query_scalar("SELECT volume_acquisition_closed_at IS NULL AND NOT EXISTS(SELECT 1 FROM run_cleanup_targets WHERE run_id=$1) FROM runs WHERE id=$1")
        .bind(fixture.command.run_id.as_uuid()).fetch_one(pool).await.expect("snapshot rollback");
    assert!(open);
}
