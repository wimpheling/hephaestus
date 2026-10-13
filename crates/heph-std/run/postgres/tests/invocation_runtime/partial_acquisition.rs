use super::{fixture::Fixture, locking::code};
use run_domain::RunState;
use run_orchestrator::RunRepository;
use sqlx::{PgPool, Postgres, Transaction};
use volume_trait::RunVolumeMetadataRepository;

#[sqlx::test(migrations = false)]
#[ignore = "requires actual private119 PostgreSQL/native ext4"]
async fn raw_single_lease_with_null_or_exact_snapshot_cannot_commit_partial_acquisition(
    pool: PgPool,
) {
    let h = Fixture::new(&pool, 2).await;
    h.consume().await.unwrap();
    h.repository()
        .transition(h.start.run_id, RunState::LeasingVolume, None, None)
        .await
        .unwrap();
    let selected = h
        .volumes()
        .load_run_selections(h.start.run_id)
        .await
        .unwrap();
    let before = h.rows().await;
    for exact_snapshot in [false, true] {
        let mut raw = h.birth.worker.begin().await.unwrap();
        if exact_snapshot {
            sqlx::query("UPDATE runs SET volume_selection_snapshot=$2 WHERE id=$1")
                .bind(h.start.run_id.as_uuid())
                .bind(serde_json::to_value(&selected).unwrap())
                .execute(&mut *raw)
                .await
                .unwrap();
        }
        let insertion = insert_single(&mut raw, h.start.run_id.as_uuid()).await;
        let error = if exact_snapshot {
            insertion.unwrap();
            let count: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1",
            )
            .bind(h.start.run_id.as_uuid())
            .fetch_one(&mut *raw)
            .await
            .unwrap();
            assert_eq!(count, 1, "the attempted incomplete two-slot set is real");
            raw.commit().await.unwrap_err()
        } else {
            let error = insertion.unwrap_err();
            raw.rollback().await.unwrap();
            error
        };
        assert_eq!(code(&error).as_deref(), Some("23000"), "{error:?}");
        assert_eq!(
            h.rows().await,
            before,
            "snapshot and ALL fences must roll back"
        );
        h.physical.assert_unchanged();
    }
    let now = time::OffsetDateTime::now_utc();
    let leases = h
        .volumes()
        .acquire_run(
            &selected,
            h.scope.host_id(),
            now,
            now + time::Duration::minutes(5),
        )
        .await
        .unwrap();
    assert_eq!(
        leases.len(),
        2,
        "legitimate whole-set acquisition still succeeds"
    );
    h.physical.assert_unchanged();
}

async fn insert_single(
    tx: &mut Transaction<'_, Postgres>,
    run: uuid::Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,acquired_at,heartbeat_at,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash)
      SELECT $2,binding.volume_id,execution.instance_id,execution.id,execution.vm_host_id,volume.lease_generation+1,'active',now(),now(),now()+interval '5 minutes',binding.provenance,execution.instance_revision_id,execution.release_id,execution.release_agent_id,binding.project_id,binding.slot_key,binding.access_mode,binding.guest_path,binding.slot_required,binding.minimum_capacity_bytes,agent.runtime_contract_hash
      FROM runs execution JOIN agent_instance_revision_volume_bindings binding ON binding.instance_revision_id=execution.instance_revision_id JOIN agent_instance_state_volumes volume ON volume.id=binding.volume_id JOIN release_agents agent ON agent.id=execution.release_agent_id
      WHERE execution.id=$1 ORDER BY binding.volume_id LIMIT 1")
        .bind(run).bind(uuid::Uuid::new_v4()).execute(&mut **tx).await?;
    Ok(())
}
