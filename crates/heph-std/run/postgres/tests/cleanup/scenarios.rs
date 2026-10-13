use run_domain::{
    RunCleanupReceipt, RunCleanupTarget, RunCleanupVmObservation, RunCleanupVmTargetKind, RunState,
};
use run_orchestrator::RunCleanupRepository;
use run_postgres::PgRunCleanupRepository;
use sqlx::PgPool;
use time::OffsetDateTime;
use vm_trait::VmId;

use super::fixture::{
    Fixture, Role, command, create_planned, insert_run, insert_selected_lease, role_pool, scope,
};

pub async fn scope_and_immutable_target(pool: &PgPool, fixture: &Fixture) {
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunCleanupRepository::new(worker.clone(), scope());
    let run = fixture.command.run_id;
    assert_actor_denied(pool, run).await;
    repository
        .bind_vm_before_provision(
            run,
            fixture.command.instance_id,
            fixture.command.instance_revision_id,
            &scope(),
            &VmId(fixture.command.run_id.to_string()),
        )
        .await
        .expect("durable planned VM before IO");
    assert!(
        repository
            .bind_vm_before_provision(
                run,
                fixture.command.instance_id,
                fixture.command.instance_revision_id,
                &scope(),
                &VmId("replacement-vm".into())
            )
            .await
            .is_err()
    );
    let wrong_scope =
        run_domain::RunCleanupHostId::new("another-owner".into(), "cleanup-host-1".into())
            .expect("other scope");
    let wrong = PgRunCleanupRepository::new(worker.clone(), wrong_scope);
    assert!(wrong.begin_cleanup(run).await.is_err());
    assert!(wrong.recorded_cleanup_receipt(run).await.is_err());
    let target = repository
        .begin_cleanup(run)
        .await
        .expect("freeze exact set");
    assert_eq!(target.leases().len(), 1);
    assert_eq!(
        repository.begin_cleanup(run).await.expect("target replay"),
        target
    );
    let digest: Vec<u8> = sqlx::query_scalar("SELECT run_cleanup_target_digest($1,1)")
        .bind(run.as_uuid())
        .fetch_one(&worker)
        .await
        .expect("independent SQL digest");
    assert_eq!(digest.as_slice(), target.digest());
    let omitted = RunCleanupTarget::new(
        run,
        target.instance_id(),
        target.revision_id(),
        target.vm_target().clone(),
        target.generation(),
        Vec::new(),
    )
    .expect("different shape");
    assert!(
        repository
            .record_cleanup_receipt(&receipt(omitted))
            .await
            .is_err()
    );
    assert!(sqlx::query("UPDATE run_cleanup_targets SET target_digest=decode(repeat('00',32),'hex') WHERE run_id=$1")
        .bind(run.as_uuid()).execute(pool).await.is_err());
    assert!(
        sqlx::query("UPDATE runs SET state='cleaned_up' WHERE id=$1")
            .bind(run.as_uuid())
            .execute(&worker)
            .await
            .is_err()
    );
}

async fn assert_actor_denied(pool: &PgPool, run: runtime_types::RunId) {
    let actor = PgRunCleanupRepository::new(role_pool(pool, Role::Actor).await, scope());
    assert!(actor.begin_cleanup(run).await.is_err());
    assert!(actor.recorded_cleanup_receipt(run).await.is_err());
}

fn receipt(target: RunCleanupTarget) -> RunCleanupReceipt {
    let observation = RunCleanupVmObservation::destroyed(
        scope(),
        target.vm_target().vm_id().expect("known VM").clone(),
    )
    .expect("observation");
    RunCleanupReceipt::new(target, observation, OffsetDateTime::now_utc()).expect("receipt shape")
}

pub async fn fences_and_completion(pool: &PgPool, fixture: &Fixture) {
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunCleanupRepository::new(worker.clone(), scope());
    let target = repository
        .begin_cleanup(fixture.command.run_id)
        .await
        .expect("target");
    let receipt = receipt(target);
    repository
        .record_cleanup_receipt(&receipt)
        .await
        .expect("trusted worker observation");
    assert_eq!(
        repository
            .recorded_cleanup_receipt(fixture.command.run_id)
            .await
            .expect("crash receipt lookup"),
        Some(receipt.clone())
    );
    assert!(
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET state='released',released_at=now() WHERE id=$1"
        )
        .bind(fixture.leases[0])
        .execute(&worker)
        .await
        .is_err(),
        "scalar release must fail at commit"
    );
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(fixture.command.run_id.as_uuid())
        .execute(&worker)
        .await
        .expect("enter cleanup");
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=8 WHERE id=$1")
        .bind(fixture.volumes[0])
        .execute(pool)
        .await
        .expect("stale current resource generation");
    assert!(repository.finish_cleanup(&receipt).await.is_err());
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM agent_instance_volume_leases WHERE id=$1",
    )
    .bind(fixture.leases[0])
    .fetch_one(pool)
    .await
    .expect("rollback evidence");
    assert!(
        !released,
        "failed whole-set completion cannot leave a partial release"
    );
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=7 WHERE id=$1")
        .bind(fixture.volumes[0])
        .execute(pool)
        .await
        .expect("restore exact fixture generation");
    assert_eq!(
        repository
            .finish_cleanup(&receipt)
            .await
            .expect("atomic cleanup")
            .state,
        RunState::CleanedUp
    );
    lease_reuse(pool, &worker, fixture).await;
    assert_eq!(
        repository
            .finish_cleanup(&receipt)
            .await
            .expect("completed receipt replay")
            .state,
        RunState::CleanedUp
    );
    let held: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_leases WHERE volume_id=$1 AND released_at IS NULL AND fencing_token=8")
        .bind(fixture.volumes[0]).fetch_one(pool).await.expect("new fence retained");
    assert_eq!(
        held, 1,
        "old completion must not release a subsequently reused resource"
    );
}

async fn lease_reuse(pool: &PgPool, worker: &PgPool, fixture: &Fixture) {
    let next = run_domain::StartRun {
        run_id: runtime_types::RunId::new(),
        command_id: runtime_types::CommandId::new(),
        ..fixture.command.clone()
    };
    insert_run(pool, &next).await;
    sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
        .bind(next.run_id.as_uuid())
        .execute(worker)
        .await
        .expect("new run admission");
    insert_selected_lease(
        worker,
        &next,
        uuid::Uuid::new_v4(),
        fixture.volumes[0],
        "cleanup-host-1",
        8,
    )
    .await;
}

pub async fn empty_and_historical(pool: &PgPool) {
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunCleanupRepository::new(worker.clone(), scope());
    let command = command();
    super::support::seed_instance(pool, &command).await;
    create_planned(pool, &command).await;
    repository
        .bind_vm_before_provision(
            command.run_id,
            command.instance_id,
            command.instance_revision_id,
            &scope(),
            &VmId(command.run_id.to_string()),
        )
        .await
        .expect("zero-volume planned ownership");
    let target = repository
        .begin_cleanup(command.run_id)
        .await
        .expect("explicit empty set");
    assert!(target.leases().is_empty());
    let receipt = receipt(target);
    repository
        .record_cleanup_receipt(&receipt)
        .await
        .expect("empty set observation");
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(command.run_id.as_uuid())
        .execute(&worker)
        .await
        .expect("empty cleanup state");
    assert!(
        sqlx::query(
            "INSERT INTO run_cleanup_completions(run_id,generation,target_digest) VALUES($1,1,$2)"
        )
        .bind(command.run_id.as_uuid())
        .bind(receipt.target().digest().as_slice())
        .execute(&worker)
        .await
        .is_err(),
        "empty completion alone cannot commit without CleanedUp"
    );
    repository
        .finish_cleanup(&receipt)
        .await
        .expect("empty atomic completion");
    let historical = run_domain::StartRun {
        run_id: runtime_types::RunId::new(),
        command_id: runtime_types::CommandId::new(),
        ..command
    };
    insert_run(pool, &historical).await;
    let unresolved = repository
        .begin_cleanup(historical.run_id)
        .await
        .expect("historical hold");
    assert_eq!(
        unresolved.vm_target().kind(),
        RunCleanupVmTargetKind::HistoricalUnresolved
    );
    assert!(
        RunCleanupReceipt::new(
            unresolved,
            RunCleanupVmObservation::authoritatively_absent(scope(), VmId("guessed-vm".into()))
                .expect("shape"),
            OffsetDateTime::now_utc()
        )
        .is_err()
    );
}

pub async fn acquisition_close_race(pool: &PgPool) {
    let command = command();
    super::support::seed_instance(pool, &command).await;
    create_planned(pool, &command).await;
    let worker = role_pool(pool, Role::Worker).await;
    let repository = PgRunCleanupRepository::new(worker.clone(), scope());
    repository
        .bind_vm_before_provision(
            command.run_id,
            command.instance_id,
            command.instance_revision_id,
            &scope(),
            &VmId(command.run_id.to_string()),
        )
        .await
        .expect("planned race VM");
    let mut acquisition = worker.begin().await.expect("acquisition transaction");
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(command.instance_id.as_uuid())
        .execute(&mut *acquisition)
        .await
        .expect("consumer first");
    sqlx::query("SELECT id FROM runs WHERE id=$1 FOR UPDATE")
        .bind(command.run_id.as_uuid())
        .execute(&mut *acquisition)
        .await
        .expect("run acquisition lock");
    let cleanup = tokio::spawn(async move { repository.begin_cleanup(command.run_id).await });
    let mut cleanup = cleanup;
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), &mut cleanup)
            .await
            .is_err(),
        "closure must wait for acquisition transaction"
    );
    acquisition
        .commit()
        .await
        .expect("finish acquisition boundary");
    cleanup
        .await
        .expect("cleanup task")
        .expect("serialized closure");
    assert!(
        sqlx::query("UPDATE runs SET volume_acquisition_closed_at=NULL WHERE id=$1")
            .bind(command.run_id.as_uuid())
            .execute(&worker)
            .await
            .is_err(),
        "closed acquisition cannot reopen"
    );
}
