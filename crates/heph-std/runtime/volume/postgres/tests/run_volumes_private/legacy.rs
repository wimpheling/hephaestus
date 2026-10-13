use runtime_types::{RunId, VolumeId};
use sqlx::PgPool;
use uuid::Uuid;
use volume_domain::VolumeSelectionOrigin;
use volume_trait::{RunVolumeMetadataRepository, VolumeError, VolumeMetadataRepository};

use crate::{fixture, support::Fixture};

pub async fn exercise(pool: &PgPool, seeded: &Fixture) {
    let revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(seeded.instance)
            .fetch_one(pool)
            .await
            .expect("exact legacy revision");
    let old = fixture::normal_run(
        pool,
        seeded,
        seeded.instance,
        revision,
        seeded.release_agent,
    )
    .await;
    let old_id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',7,'active',now()+interval '1 hour')")
        .bind(old_id).bind(seeded.volume).bind(seeded.instance).bind(old.as_uuid()).execute(pool).await.expect("real pre108 lease");
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=7 WHERE id=$1")
        .bind(seeded.volume)
        .execute(pool)
        .await
        .expect("old fence");
    let checksums: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("published107 history");
    let historical = unproven(pool, seeded, revision).await;
    let unknown = super::historical_update::seed(pool, seeded).await;
    fixture::apply_private(pool).await;
    super::historical_update::exercise(pool, unknown).await;
    let after: Vec<(i64, Vec<u8>)> = sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("history after private SQL");
    assert_eq!(
        checksums, after,
        "private rehearsal never rewrites published migrations"
    );
    old_evidence(pool, seeded, old, historical, old_id).await;
    super::scalar::exercise(pool, seeded).await;
    new_origin_and_update(pool, seeded).await;
    println!(
        "PRIVATE_108_DB_ONLY=1 legacy_origin_without_binding=1 exact_run_over_command_flag=1 historical_fences_preserved=1 unproven_new_acquisition_denied=1 legacy_candidate_hook=1 no_vm_dispatch_claim=1"
    );
}

async fn old_evidence(
    pool: &PgPool,
    seeded: &Fixture,
    old: RunId,
    historical: RunId,
    old_id: Uuid,
) {
    let worker = fixture::worker(pool).await;
    let old_set = worker
        .load_run_selections(old)
        .await
        .expect("exact backfilled legacy source");
    assert_eq!(
        old_set.selections()[0].origin(),
        VolumeSelectionOrigin::LegacyOrigin
    );
    let (now, expiry) = fixture::times();
    let replay = worker
        .acquire_run(&old_set, "host", now, expiry)
        .await
        .expect("exact complete old acquisition replay");
    assert_eq!(replay[0].lease().id.as_uuid(), old_id);
    assert_eq!(replay[0].lease().fencing_token, 7);
    assert!(matches!(
        worker.leases_for_run(old, "foreign").await,
        Err(VolumeError::WrongHost { .. })
    ));
    let evidence = worker
        .leases_for_run(historical, "host")
        .await
        .expect("global historical set");
    assert_eq!(evidence.len(), 1);
    assert!(evidence[0].selection().is_none());
    let bad=sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at,selection_origin) VALUES(gen_random_uuid(),$1,$2,$3,'host',8,'active',now()+interval '1 hour','historical_unproven')")
        .bind(seeded.volume).bind(seeded.instance).bind(old.as_uuid()).execute(pool).await.expect_err("new historical evidence is forbidden");
    assert_eq!(
        bad.as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    assert!(
        worker
            .active_lease_for_run(historical, "host")
            .await
            .is_err()
    );
}

async fn unproven(pool: &PgPool, seeded: &Fixture, _revision: Uuid) -> RunId {
    let (instance, revision, volume) = consumer(pool, seeded).await;
    let run = fixture::normal_run(pool, seeded, instance, revision, seeded.release_agent).await;
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES(gen_random_uuid(),$1,$2,$3,'host',11,'active',now()+interval '1 hour')")
        .bind(volume).bind(instance).bind(run.as_uuid()).execute(pool).await.expect("ambiguous pre108 history");
    sqlx::query("UPDATE agent_instance_state_volumes SET lease_generation=11 WHERE id=$1")
        .bind(volume)
        .execute(pool)
        .await
        .expect("held historical fence");
    sqlx::query("UPDATE agent_instances SET state_volume_id=NULL WHERE id=$1")
        .bind(instance)
        .execute(pool)
        .await
        .expect("historical pointer is no longer proven");
    run
}

async fn new_origin_and_update(pool: &PgPool, seeded: &Fixture) {
    let (instance, revision, volume) = consumer(pool, seeded).await;
    let bindings:i64=sqlx::query_scalar("SELECT count(*) FROM agent_instance_revision_volume_bindings WHERE instance_revision_id=$1")
        .bind(revision).fetch_one(pool).await.expect("no fabricated binding");
    assert_eq!(bindings, 0);
    let run = fixture::normal_run(pool, seeded, instance, revision, seeded.release_agent).await;
    let worker = fixture::worker(pool).await;
    let selected = worker
        .load_run_selections(run)
        .await
        .expect("new checked origin");
    assert_eq!(
        selected.selections()[0].scope().volume_id(),
        VolumeId::from_uuid(volume)
    );
    let (now, expiry) = fixture::times();
    let leases = worker
        .acquire_run(&selected, "host", now, expiry)
        .await
        .expect("atomic canonical legacy acquisition");
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].lease().fencing_token, 1);
    assert_eq!(
        worker
            .acquire_run(&selected, "host", now, expiry)
            .await
            .expect("same set replay"),
        leases
    );
    worker
        .mark_run_attached(&selected, "host", now, expiry)
        .await
        .expect("complete attachment");
    worker
        .heartbeat_run(&selected, "host", now, expiry)
        .await
        .expect("complete heartbeat");
    let worker_pool = fixture::worker_pool(pool).await;
    sqlx::query("UPDATE runs SET state='cleaning_up' WHERE id=$1")
        .bind(run.as_uuid())
        .execute(&worker_pool)
        .await
        .expect("durable acquisition closure");
    assert!(
        worker
            .acquire_run(&selected, "host", now, expiry)
            .await
            .is_err()
    );
    assert!(
        worker
            .heartbeat_run(&selected, "host", now, expiry)
            .await
            .is_err()
    );
    assert_eq!(
        fixture::count(pool, run).await,
        1,
        "closure does not prove detach or release"
    );
    hook(pool, seeded).await;
}

async fn hook(pool: &PgPool, seeded: &Fixture) {
    let (instance, revision, _) = consumer(pool, seeded).await;
    let candidate = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,false FROM agent_instance_revisions WHERE id=$2")
        .bind(candidate).bind(revision).execute(pool).await.expect("exact nonrunnable hook candidate");
    let run = RunId::new();
    let update = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_updates(id,instance_id,expected_current_revision_id,candidate_revision_id,state,actor_id) VALUES($1,$2,$3,$4,'candidate',$5)")
        .bind(update).bind(instance).bind(revision).bind(candidate).bind(seeded.maintainer.as_uuid()).execute(pool).await.expect("admitted update candidate");
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,'update',$6,'leasing_volume',false,now(),now())")
        .bind(run.as_uuid()).bind(instance).bind(candidate).bind(seeded.release).bind(seeded.release_agent).bind(Uuid::new_v4()).execute(pool).await.expect("exact hook run");
    sqlx::query("UPDATE agent_updates SET state='hook_running',hook_run_id=$2 WHERE id=$1")
        .bind(update)
        .bind(run.as_uuid())
        .execute(pool)
        .await
        .expect("hook admitted");
    sqlx::query("UPDATE agent_instances SET state='updating',run_gate_open=false WHERE id=$1")
        .bind(instance)
        .execute(pool)
        .await
        .expect("legacy update closed normal gate");
    let worker = fixture::worker(pool).await;
    let selected = worker
        .load_run_selections(run)
        .await
        .expect("exact admitted hook, not active normal revision");
    assert_eq!(selected.identity().revision_id().as_uuid(), candidate);
    let (now, expiry) = fixture::times();
    let volume = selected.selections()[0].scope().volume_id();
    let lease = worker
        .acquire(volume, run, "host", now, expiry)
        .await
        .expect("scalar exact admitted candidate hook");
    worker
        .mark_attached(&lease, now, expiry)
        .await
        .expect("scalar hook attach");
    worker
        .heartbeat(&lease, now, expiry)
        .await
        .expect("scalar hook heartbeat");
    super::scalar::hook_callback(pool, seeded, instance, update, run, &lease).await;
}

pub async fn consumer(pool: &PgPool, seeded: &Fixture) -> (Uuid, Uuid, Uuid) {
    let instance = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let volume = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state) SELECT $1,project_id,family_id,$3,'active' FROM agent_instances WHERE id=$2")
        .bind(instance).bind(seeded.instance).bind(format!("legacy-{instance}")).execute(pool).await.expect("new ordinary legacy consumer");
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$3)")
        .bind(revision).bind(instance).bind(seeded.instance).execute(pool).await.expect("ordinary revision without typed bindings");
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,host_id,host_path,capacity_bytes,filesystem_uuid,state,provisioning_state) VALUES($1,$2,$3,'host',$4,16777216,gen_random_uuid(),'ready','ready')")
        .bind(volume).bind(instance).bind(seeded.consuming_project).bind(format!("/fixture/{volume}.raw")).execute(pool).await.expect("same-origin ready metadata, no backing claim");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2,state_volume_id=$3 WHERE id=$1")
        .bind(instance)
        .bind(revision)
        .bind(volume)
        .execute(pool)
        .await
        .expect("actual legacy scalar pointer");
    (instance, revision, volume)
}
