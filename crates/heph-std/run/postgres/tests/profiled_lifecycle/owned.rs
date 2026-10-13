use run_domain::{FreshRunExecutionProfile, RunState};
use run_orchestrator::RunRepository;
use run_postgres::{PgRunRepository, ProfiledRunRecoveryHoldReason};
use sqlx::PgPool;
use vm_trait::VmId;
use volume_trait::RunVolumeMetadataRepository;

use super::{
    fixture::{self, Fixture},
    public_fixture::scope,
};

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn owned_zero_set_uses_actual_plan_and_wrong_scope_cannot_bind_or_recover(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, false).await;
    let owned = PgRunRepository::new(f.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(scope());
    let vm = VmId(f.command.run_id.to_string());
    owned
        .create_run_with_vm_plan(&f.command, &scope(), &vm)
        .await
        .unwrap();
    owned
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        owned
            .bind_resources(f.command.run_id, None, None, None, &vm.0)
            .await
            .is_err(),
        "no frozen complete set yet"
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    let selections = f
        .volumes
        .load_run_selections(f.command.run_id)
        .await
        .unwrap();
    assert!(selections.selections().is_empty());
    let now = time::OffsetDateTime::now_utc();
    assert!(
        f.volumes
            .acquire_run(
                &selections,
                scope().host_id(),
                now,
                now + time::Duration::minutes(5)
            )
            .await
            .unwrap()
            .is_empty()
    );
    owned
        .bind_resources(f.command.run_id, None, None, None, &vm.0)
        .await
        .unwrap();
    let foreign = owned.clone().with_vm_owner_scope(
        run_domain::RunCleanupHostId::new("other-provider".into(), scope().host_id().into())
            .unwrap(),
    );
    let before = fixture::evidence(&pool, f.command.run_id).await;
    assert!(
        foreign
            .transition(f.command.run_id, RunState::Provisioning, None, None)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(&pool, f.command.run_id).await, before);
    assert!(foreign.recoverable_runs().await.unwrap().is_empty());
    assert_eq!(
        foreign.profiled_recovery_holds().await.unwrap()[0].reason,
        ProfiledRunRecoveryHoldReason::OwnedProviderScopeUnresolved
    );
    owned
        .transition(f.command.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    assert_eq!(
        owned.recoverable_runs().await.unwrap()[0].id,
        f.command.run_id
    );
    owned
        .transition(
            f.command.run_id,
            RunState::Failed,
            None,
            Some("controlled failure"),
        )
        .await
        .unwrap();
    owned
        .transition(f.command.run_id, RunState::CleaningUp, None, None)
        .await
        .unwrap();
    assert!(
        owned
            .transition(f.command.run_id, RunState::CleanedUp, None, None)
            .await
            .is_err(),
        "owned cleanup requires actual persisted canonical completion even for zero leases"
    );
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable public107 PostgreSQL"]
async fn unconfigured_lifecycle_still_uses_only_public107_columns(pool: PgPool) {
    super::public_fixture::checkpoint(107)
        .run(&pool)
        .await
        .unwrap();
    let f = Fixture::new(&pool, false).await;
    let default = PgRunRepository::new(f.worker.clone());
    default.create_run(&f.command).await.unwrap();
    default
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    default
        .bind_resources(
            f.command.run_id,
            None,
            None,
            None,
            &f.command.run_id.to_string(),
        )
        .await
        .unwrap();
    default
        .transition(f.command.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    assert_eq!(
        default.recoverable_runs().await.unwrap()[0].id,
        f.command.run_id
    );
    assert!(default.profiled_recovery_holds().await.is_err());
    let private_column:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM information_schema.columns WHERE table_name='runs' AND column_name='execution_profile')")
        .fetch_one(&pool).await.unwrap();
    assert!(!private_column);
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn owned_scalar_projection_requires_exact_frozen_singleton_legacy_origin(pool: PgPool) {
    fixture::schema(&pool).await;
    let f = Fixture::new(&pool, true).await;
    let owner =
        run_domain::RunCleanupHostId::new("controlled-owned-root".into(), "lifecycle-host".into())
            .unwrap();
    let owned = PgRunRepository::new(f.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(owner.clone());
    let vm = VmId(f.command.run_id.to_string());
    owned
        .create_run_with_vm_plan(&f.command, &owner, &vm)
        .await
        .unwrap();
    owned
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selections = f
        .volumes
        .load_run_selections(f.command.run_id)
        .await
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let held = f
        .volumes
        .acquire_run(
            &selections,
            owner.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(held.len(), 1);
    let lease = held[0].lease();
    assert!(
        owned
            .bind_resources(f.command.run_id, None, None, None, &vm.0)
            .await
            .is_err()
    );
    owned
        .bind_resources(
            f.command.run_id,
            Some(lease.volume_id),
            Some(lease.id),
            Some(lease.fencing_token),
            &vm.0,
        )
        .await
        .unwrap();
    let bound = owned.get(f.command.run_id).await.unwrap();
    assert_eq!(bound.lease_id, Some(lease.id));
    assert!(
        owned
            .bind_resources(f.command.run_id, None, None, None, &vm.0)
            .await
            .is_err(),
        "immutable projection cannot disappear"
    );
    owned
        .transition(f.command.run_id, RunState::Provisioning, None, None)
        .await
        .unwrap();
    f.worker.close().await;
}

#[sqlx::test(migrations = false)]
#[ignore = "requires actual disposable private lifecycle PostgreSQL"]
async fn raw_same_count_snapshot_path_or_named_origin_substitution_cannot_authorize_io(
    pool: PgPool,
) {
    fixture::schema(&pool).await;
    for substitute_origin in [false, true] {
        assert_snapshot_substitution(&pool, substitute_origin).await;
    }
}

async fn assert_snapshot_substitution(pool: &PgPool, substitute_origin: bool) {
    let f = Fixture::new(pool, true).await;
    let owner =
        run_domain::RunCleanupHostId::new("controlled-owned-root".into(), "lifecycle-host".into())
            .unwrap();
    let repository = PgRunRepository::new(f.worker.clone())
        .with_execution_profile(FreshRunExecutionProfile::OwnedCanonical)
        .with_vm_owner_scope(owner.clone());
    let vm = VmId(f.command.run_id.to_string());
    repository
        .create_run_with_vm_plan(&f.command, &owner, &vm)
        .await
        .unwrap();
    repository
        .transition(f.command.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = f
        .volumes
        .load_run_selections(f.command.run_id)
        .await
        .unwrap();
    let mut snapshot = serde_json::to_value(&selected).unwrap();
    assert_eq!(snapshot["selections"].as_array().unwrap().len(), 1);
    if substitute_origin {
        snapshot["selections"][0]["origin"] = serde_json::json!("explicit");
    } else {
        snapshot["selections"][0]["declaration"]["guest_path"] = serde_json::json!("/substituted");
    }
    sqlx::query("UPDATE runs SET volume_selection_snapshot=$2 WHERE id=$1")
        .bind(f.command.run_id.as_uuid())
        .bind(snapshot)
        .execute(&f.worker)
        .await
        .unwrap();
    // Raw trusted-worker insertion still passes the existing exact lease
    // catalog/UUID/fence guards. The frozen snapshot has the same cardinality,
    // but differs from the normalized lease; neither core bind nor raw phase
    // transition may treat its count as authority.
    let lease = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash) SELECT $1,consumer.state_volume_id,execution.instance_id,execution.id,'lifecycle-host',1,'active',now()+interval '5 minutes','legacy_origin',execution.instance_revision_id,execution.release_id,execution.release_agent_id,consumer.project_id,'state','read_write','/var/lib/hephaestus',true,1,agent.runtime_contract_hash FROM runs execution JOIN agent_instances consumer ON consumer.id=execution.instance_id JOIN release_agents agent ON agent.id=execution.release_agent_id WHERE execution.id=$2")
        .bind(lease).bind(f.command.run_id.as_uuid()).execute(&f.worker).await.unwrap();
    let before = fixture::evidence(pool, f.command.run_id).await;
    assert!(
        repository
            .bind_resources(
                f.command.run_id,
                f.volume,
                Some(runtime_types::LeaseId::from_uuid(lease)),
                Some(1),
                &vm.0
            )
            .await
            .is_err()
    );
    assert!(
        sqlx::query("UPDATE runs SET state='provisioning' WHERE id=$1")
            .bind(f.command.run_id.as_uuid())
            .execute(&f.worker)
            .await
            .is_err()
    );
    assert_eq!(fixture::evidence(pool, f.command.run_id).await, before);
    f.worker.close().await;
}
