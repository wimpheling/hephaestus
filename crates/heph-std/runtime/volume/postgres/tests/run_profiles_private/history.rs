use sqlx::PgPool;
use uuid::Uuid;

use crate::support::Fixture;

pub async fn update_only(pool: &PgPool, f: &Fixture) -> (Uuid, serde_json::Value) {
    let volume = Uuid::new_v4();
    let lease = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let update = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state) SELECT $1,project_id,family_id,$2,'active' FROM agent_instances WHERE id=$3")
        .bind(instance).bind(instance.to_string()).bind(f.instance).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$3)")
        .bind(revision).bind(instance).bind(f.instance).execute(pool).await.unwrap();
    let candidate = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,false FROM agent_instance_revisions WHERE id=$2")
        .bind(candidate).bind(revision).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO agent_updates(id,instance_id,expected_current_revision_id,candidate_revision_id,state,actor_id) VALUES($1,$2,$3,$4,'candidate',$5)")
        .bind(update).bind(instance).bind(revision).bind(candidate).bind(f.maintainer.as_uuid()).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,host_id,host_path,capacity_bytes,filesystem_uuid,state,lease_generation) VALUES($1,$2,$3,'host',$4,16777216,gen_random_uuid(),'attached',11)")
        .bind(volume).bind(instance).bind(f.consuming_project).bind(format!("/fixture/{volume}.raw")).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,update_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',11,'active',now()+interval '1 hour')")
        .bind(lease).bind(volume).bind(instance).bind(update).execute(pool).await.unwrap();
    let row =
        sqlx::query_scalar("SELECT to_jsonb(l) FROM agent_instance_volume_leases l WHERE id=$1")
            .bind(lease)
            .fetch_one(pool)
            .await
            .unwrap();
    (lease, row)
}

pub async fn assert_held(pool: &PgPool, (lease, before): (Uuid, serde_json::Value)) {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    assert!(
        sqlx::query(
            "UPDATE agent_instance_volume_leases SET released_at=now(),state='released' WHERE id=$1"
        )
        .bind(lease)
        .execute(&mut *tx)
        .await
        .is_err()
    );
    tx.rollback().await.unwrap();
    let after=sqlx::query_scalar::<_,serde_json::Value>("SELECT to_jsonb(l)-'selection_origin'-'selected_revision_id'-'selected_release_id'-'selected_release_agent_id'-'selected_project_id'-'slot_key'-'access_mode'-'guest_path'-'slot_required'-'minimum_capacity_bytes'-'release_contract_hash'-'named_binding_revision_id' FROM agent_instance_volume_leases l WHERE id=$1").bind(lease).fetch_one(pool).await.unwrap();
    assert_eq!(before, after);
    assert!(after["run_id"].is_null());
    assert_eq!(after["fencing_token"], 11);
}
