use authz_postgres::{AUTHORIZATION_MODEL_VERSION, begin_actor_transaction};
use runtime_types::RunId;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

use crate::support::{Fixture, identity};

pub async fn consumer(pool: &PgPool, fixture: &Fixture, count: usize) -> (Uuid, Uuid, Uuid) {
    consumer_with_resources(pool, fixture, count, None).await
}

pub async fn reuse_consumer(
    pool: &PgPool,
    fixture: &Fixture,
    volumes: &[Uuid],
) -> (Uuid, Uuid, Uuid) {
    consumer_with_resources(pool, fixture, volumes.len(), Some(volumes)).await
}

async fn consumer_with_resources(
    pool: &PgPool,
    fixture: &Fixture,
    count: usize,
    existing: Option<&[Uuid]>,
) -> (Uuid, Uuid, Uuid) {
    let agent = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let slots=(0..count).map(|index|json!({"slot":format!("slot{index:02}"),"guest_path":format!("/volumes/slot{index:02}"),"access_mode":if index%2==0 {"read_only"}else{"read_write"},"required":true,"minimum_capacity_bytes":16_777_216})).collect::<Vec<_>>();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,release_id,family_id,$3,'DB-only named fixture',$4,$5,false FROM release_agents WHERE id=$2")
        .bind(agent).bind(fixture.release_agent).bind(format!("named-{agent}")).bind(json!({"volume_slots":slots})).bind([42_u8;32].as_slice()).execute(pool).await.expect("actual released fixture with immutable declarations");
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state,volume_mode,run_gate_open) SELECT $1,project_id,family_id,$3,'active','named',false FROM agent_instances WHERE id=$2")
        .bind(instance).bind(fixture.instance).bind(format!("named-{instance}")).execute(pool).await.expect("actual named consumer, no scalar pointer");
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,$3,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$4)")
        .bind(revision).bind(instance).bind(agent).bind(fixture.instance).execute(pool).await.expect("immutable selected revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(instance)
        .bind(revision)
        .execute(pool)
        .await
        .expect("exact revision, named dispatch remains closed");
    for index in 0..count {
        let volume = existing.map_or_else(Uuid::new_v4, |volumes| volumes[index]);
        let slot = format!("slot{index:02}");
        let path = format!("/volumes/slot{index:02}");
        let mode = if index % 2 == 0 {
            "read_only"
        } else {
            "read_write"
        };
        if existing.is_none() {
            sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,host_id,host_path,capacity_bytes,filesystem_uuid,state,provisioning_state) VALUES($1,$2,'host',$3,16777216,gen_random_uuid(),'ready','ready')")
                .bind(volume).bind(fixture.consuming_project).bind(format!("/fixture/{volume}.raw")).execute(pool).await.expect("simulated ready provider metadata, no host I/O");
        }
        sqlx::query("INSERT INTO agent_instance_revision_volume_bindings(instance_revision_id,instance_id,project_id,release_agent_id,slot_key,volume_id,access_mode,guest_path,slot_required,minimum_capacity_bytes,provenance,created_by) VALUES($1,$2,$3,$4,$5,$6,$7,$8,true,16777216,'explicit',$9)")
            .bind(revision).bind(instance).bind(fixture.consuming_project).bind(agent).bind(&slot).bind(volume).bind(mode).bind(path).bind(fixture.maintainer.as_uuid()).execute(pool).await.expect("exact authoritative binding");
        let actor = identity(fixture.maintainer);
        let mut tx = begin_actor_transaction(pool, &actor)
            .await
            .expect("real actor context");
        sqlx::query("SET LOCAL ROLE hephaestus_app")
            .execute(&mut *tx)
            .await
            .expect("ordinary actor role");
        sqlx::query("INSERT INTO agent_instance_volume_mount_grants(id,instance_revision_id,instance_id,release_agent_id,slot_key,volume_id,access_mode,release_contract_hash,created_by,request_id,authorization_model_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
            .bind(Uuid::new_v4()).bind(revision).bind(instance).bind(agent).bind(slot).bind(volume).bind(mode).bind([42_u8;32].as_slice()).bind(actor.user_id.as_uuid()).bind(actor.request_id.as_uuid()).bind(AUTHORIZATION_MODEL_VERSION).execute(&mut *tx).await.expect("live project-maintainer source authorizes exact grant");
        tx.commit().await.expect("real typed grant commit");
    }
    (instance, revision, agent)
}

pub async fn try_run(
    pool: &PgPool,
    fixture: &Fixture,
    instance: Uuid,
    revision: Uuid,
    agent: Uuid,
) -> Result<RunId, sqlx::Error> {
    let supported: bool = sqlx::query_scalar("SELECT named_volume_dispatch_supported($1)")
        .bind(instance)
        .fetch_one(pool)
        .await?;
    if supported {
        sqlx::query("UPDATE agent_instances SET run_gate_open=true WHERE id=$1")
            .bind(instance)
            .execute(pool)
            .await?;
    }
    let attachment = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy,created_by) VALUES($1,$2,$3,$4,'refs/heads/main','manual',$5) ON CONFLICT(instance_id,repository_id,ref_selector) DO NOTHING")
        .bind(attachment).bind(instance).bind(fixture.consuming_project).bind(fixture.consuming_repository).bind(fixture.maintainer.as_uuid()).execute(pool).await?;
    let attachment:Uuid=sqlx::query_scalar("SELECT id FROM agent_attachments WHERE instance_id=$1 AND repository_id=$2 AND ref_selector='refs/heads/main'")
        .bind(instance).bind(fixture.consuming_repository).fetch_one(pool).await?;
    let run = RunId::new();
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'normal',$7,'leasing_volume',false,now(),now())")
        .bind(run.as_uuid()).bind(instance).bind(revision).bind(fixture.release).bind(agent).bind(attachment).bind(Uuid::new_v4()).execute(pool).await?;
    Ok(run)
}
