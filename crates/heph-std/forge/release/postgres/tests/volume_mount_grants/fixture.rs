use crate::support::Fixture;
use capability_domain::{AuthorityHash, CapabilitySlotKey};
use runtime_types::{AgentInstanceId, AgentInstanceRevisionId, ReleaseAgentId, RunId, VolumeId};
use sqlx::{PgPool, postgres::PgPoolOptions};
use uuid::Uuid;
use volume_domain::{VolumeAccessMode, VolumeMountScope};

#[derive(Clone, Copy)]
pub enum Role {
    Actor,
    Worker,
}

pub async fn role_pool(pool: &PgPool, role: Role) -> PgPool {
    PgPoolOptions::new()
        .max_connections(2)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                match role {
                    Role::Actor => sqlx::query("SET ROLE hephaestus_app"),
                    Role::Worker => sqlx::query("SET ROLE hephaestus_worker"),
                }
                .execute(connection)
                .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .expect("normal role pool")
}

pub async fn legacy(pool: &PgPool, fixture: &Fixture) -> (VolumeMountScope, RunId) {
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(fixture.instance)
        .bind(fixture.volume)
        .execute(pool)
        .await
        .expect("legacy pointer");
    let revision: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id=$1")
            .bind(fixture.instance)
            .fetch_one(pool)
            .await
            .expect("legacy active revision");
    sqlx::query("INSERT INTO agent_instance_revision_volume_bindings(instance_revision_id,instance_id,project_id,release_agent_id,slot_key,volume_id,access_mode,guest_path,slot_required,minimum_capacity_bytes,provenance,created_by) VALUES($1,$2,$3,$4,'state',$5,'read_write','/var/lib/hephaestus',true,1,'legacy_state',$6)")
        .bind(revision).bind(fixture.instance).bind(fixture.consuming_project).bind(fixture.release_agent).bind(fixture.volume).bind(fixture.maintainer.as_uuid()).execute(pool).await.expect("immutable legacy evidence");
    let scope = VolumeMountScope::new(
        AgentInstanceId::from_uuid(fixture.instance),
        AgentInstanceRevisionId::from_uuid(revision),
        ReleaseAgentId::from_uuid(fixture.release_agent),
        CapabilitySlotKey::parse("state").unwrap(),
        VolumeId::from_uuid(fixture.volume),
        VolumeAccessMode::ReadWrite,
        AuthorityHash::from_bytes([4; 32]),
    )
    .unwrap();
    let run = normal_run(pool, fixture, &scope).await;
    (scope, run)
}

pub async fn explicit(pool: &PgPool, fixture: &Fixture) -> (VolumeMountScope, RunId) {
    let agent = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let volume = Uuid::new_v4();
    sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,release_id,family_id,'explicit','Explicit',jsonb_build_object('volume_slots',jsonb_build_array(jsonb_build_object('slot','data','guest_path','/data','access_mode','read_only','required',true,'minimum_capacity_bytes',16777216))),$3,false FROM release_agents WHERE id=$2")
        .bind(agent).bind(fixture.release_agent).bind([9_u8;32].as_slice()).execute(pool).await.expect("new published fixture declaration, original hash untouched");
    sqlx::query("INSERT INTO agent_instances(id,project_id,family_id,name,state) SELECT $1,project_id,family_id,'explicit','active' FROM agent_instances WHERE id=$2")
        .bind(instance).bind(fixture.instance).execute(pool).await.expect("explicit consumer");
    sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,$2,$3,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=(SELECT active_revision_id FROM agent_instances WHERE id=$4)")
        .bind(revision).bind(instance).bind(agent).bind(fixture.instance).execute(pool).await.expect("explicit frozen revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
        .bind(instance)
        .bind(revision)
        .execute(pool)
        .await
        .expect("fixture activation");
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,project_id,state,capacity_bytes,filesystem_uuid) VALUES($1,$2,'uninitialized',16777216,gen_random_uuid())")
        .bind(volume).bind(fixture.consuming_project).execute(pool).await.expect("standalone resource");
    sqlx::query("INSERT INTO agent_instance_revision_volume_bindings(instance_revision_id,instance_id,project_id,release_agent_id,slot_key,volume_id,access_mode,guest_path,slot_required,minimum_capacity_bytes,provenance,created_by) VALUES($1,$2,$3,$4,'data',$5,'read_only','/data',true,16777216,'explicit',$6)")
        .bind(revision).bind(instance).bind(fixture.consuming_project).bind(agent).bind(volume).bind(fixture.maintainer.as_uuid()).execute(pool).await.expect("declared explicit evidence");
    let scope = VolumeMountScope::new(
        AgentInstanceId::from_uuid(instance),
        AgentInstanceRevisionId::from_uuid(revision),
        ReleaseAgentId::from_uuid(agent),
        CapabilitySlotKey::parse("data").unwrap(),
        VolumeId::from_uuid(volume),
        VolumeAccessMode::ReadOnly,
        AuthorityHash::from_bytes([9; 32]),
    )
    .unwrap();
    let run = normal_run(pool, fixture, &scope).await;
    (scope, run)
}

async fn normal_run(pool: &PgPool, fixture: &Fixture, scope: &VolumeMountScope) -> RunId {
    let attachment = if scope.instance_id().as_uuid() == fixture.instance {
        fixture.attachment
    } else {
        Uuid::new_v4()
    };
    if scope.instance_id().as_uuid() != fixture.instance {
        sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy,created_by) VALUES($1,$2,$3,$4,'refs/heads/main','manual',$5)")
        .bind(attachment).bind(scope.instance_id().as_uuid()).bind(fixture.consuming_project).bind(fixture.consuming_repository).bind(fixture.maintainer.as_uuid()).execute(pool).await.expect("normal run attachment");
    }
    let id = RunId::new();
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'normal',$7,'leasing_volume',true,now(),now())")
        .bind(id.as_uuid()).bind(scope.instance_id().as_uuid()).bind(scope.revision_id().as_uuid()).bind(fixture.release).bind(scope.release_agent_id().as_uuid()).bind(attachment).bind(Uuid::new_v4()).execute(pool).await.expect("exact normal preflight run");
    id
}
