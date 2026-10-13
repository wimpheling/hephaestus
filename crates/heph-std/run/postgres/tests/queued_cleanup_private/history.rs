use super::{
    admission,
    observations::{code, held, rows},
    queued_fixture::Queued,
};
use run_orchestrator::RunRepository;
use sqlx::PgPool;

#[sqlx::test(migrations = false)]
async fn reachable_foreign_canonical_ids_and_destroyed_history_deny_globally(pool: PgPool) {
    let h = Queued::new(&pool, 2, true).await;
    h.prepare().await;
    let before = rows(&pool).await;
    for session in [false, true] {
        let mut tx = pool.begin().await.unwrap();
        let snapshot = if session {
            uuid::Uuid::new_v4()
        } else {
            h.request.run_id().as_uuid()
        };
        // Denial-only foreign metadata: actual unrelated seeded Update FK tuple,
        // with every constraint enabled. Never a qualified own credential birth.
        sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) SELECT $1,id,instance_id,instance_revision_id,'foreign-history',decode(repeat('07',32),'hex') FROM runs WHERE id=$2")
            .bind(snapshot).bind(h.fixture.birth.fixture.run).execute(&mut *tx).await.unwrap();
        if session {
            sqlx::query("INSERT INTO runtime_authority_sessions(id,snapshot_id,run_id,instance_id,instance_revision_id,attachment_id,identity_hash,snapshot_hash,issuance_generation,credential_hash,status,issued_at,expires_at) SELECT $1,$2,r.id,r.instance_id,r.instance_revision_id,r.attachment_id,decode(repeat('08',32),'hex'),s.normalized_hash,1,decode(repeat('09',32),'hex'),'expired',now()-interval '2 hours',now()-interval '1 hour' FROM runs r JOIN run_authorization_snapshots s ON s.run_id=r.id AND s.id=$2 WHERE r.id=$3")
                .bind(h.request.run_id().as_uuid()).bind(snapshot).bind(h.fixture.birth.fixture.run).execute(&mut *tx).await.unwrap();
        }
        // The loader/owner observes fixture lineage before worker-only guard checks.
        let lineage:bool=sqlx::query_scalar("SELECT qualified_invocation_graph_matches(receipt) FROM agent_instance_execution_invocations receipt WHERE run_id=$1")
            .bind(h.request.run_id().as_uuid()).fetch_one(&mut *tx).await.unwrap();
        assert!(lineage);
        sqlx::query("SET LOCAL ROLE hephaestus_worker")
            .execute(&mut *tx)
            .await
            .unwrap();
        let error = admission::queued_lock(&mut tx, &h, false)
            .await
            .unwrap_err();
        assert_eq!(code(&error).as_deref(), Some("P1180"));
        tx.rollback().await.unwrap();
        assert_eq!(rows(&pool).await, before);
    }
    own_birth_denials(&h).await;
    sqlx::query("INSERT INTO secret_runtime_mounts(run_id,opaque_directory,state) VALUES($1,$2,'materialized')")
        .bind(h.request.run_id().as_uuid()).bind(uuid::Uuid::new_v4()).execute(&pool).await.unwrap();
    let materialized = rows(&pool).await;
    held(&h.repository().get(h.request.run_id()).await.unwrap_err());
    assert_eq!(rows(&pool).await, materialized);
    sqlx::query(
        "UPDATE secret_runtime_mounts SET state='destroyed',destroyed_at=now() WHERE run_id=$1",
    )
    .bind(h.request.run_id().as_uuid())
    .execute(&pool)
    .await
    .unwrap();
    let destroyed = rows(&pool).await;
    held(
        &h.repository()
            .prepare_cleanup_wake(&h.request, &h.fixture.scope)
            .await
            .unwrap_err(),
    );
    assert_eq!(rows(&pool).await, destroyed);
    h.fixture.physical.assert_unchanged();
}
async fn own_birth_denials(h: &Queued) {
    let before = rows(&h.fixture.pool).await;
    let mut tx = h.fixture.birth.worker.begin().await.unwrap();
    let error=sqlx::query("INSERT INTO run_authorization_snapshots(id,run_id,instance_id,instance_revision_id,authorization_model_version,normalized_hash) VALUES($1,$1,$2,$3,'own-pre117',decode(repeat('07',32),'hex'))")
        .bind(h.request.run_id().as_uuid()).bind(h.request.instance_id().as_uuid()).bind(h.fixture.start.instance_revision_id.as_uuid())
        .execute(&mut *tx).await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("P1190"));
    tx.rollback().await.unwrap();
    assert_eq!(rows(&h.fixture.pool).await, before);
    let mut tx = h.fixture.birth.worker.begin().await.unwrap();
    let error=sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,acquired_at,heartbeat_at,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash) SELECT gen_random_uuid(),b.volume_id,r.instance_id,r.id,r.vm_host_id,v.lease_generation+1,'active',now(),now(),now()+interval '5 minutes',b.provenance,r.instance_revision_id,r.release_id,r.release_agent_id,b.project_id,b.slot_key,b.access_mode,b.guest_path,b.slot_required,b.minimum_capacity_bytes,a.runtime_contract_hash FROM runs r JOIN agent_instance_revision_volume_bindings b ON b.instance_revision_id=r.instance_revision_id JOIN agent_instance_state_volumes v ON v.id=b.volume_id JOIN release_agents a ON a.id=r.release_agent_id WHERE r.id=$1 ORDER BY b.volume_id LIMIT 1")
        .bind(h.request.run_id().as_uuid()).execute(&mut *tx).await.unwrap_err();
    assert_eq!(code(&error).as_deref(), Some("P1190"));
    tx.rollback().await.unwrap();
    assert_eq!(rows(&h.fixture.pool).await, before);
    // Own released/foreign/expired lease history is guard-unreachable for this
    // genuine115 Run. No protected state, old receipts or producers are forged.
}
