use std::borrow::Cow;

use run_domain::{RunCleanupHostId, RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use serde_json::Value;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use uuid::Uuid;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../migrations");

// Private proof only: replace these includes with checkpoint(108) after ordered
// publication. Applied historical migration sources remain byte-identical.
pub async fn apply_private_checkpoint(pool: &PgPool) {
    sqlx::raw_sql(include_str!("/var/tmp/0108_run_volume_leases.sql"))
        .execute(pool)
        .await
        .expect("private metadata checkpoint");
    sqlx::raw_sql(include_str!("/var/tmp/0108_run_cleanup_fragment.sql"))
        .execute(pool)
        .await
        .expect("private cleanup checkpoint");
}

pub fn checkpoint(version: i64) -> Migrator {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|migration| migration.version <= version)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
}

pub fn command() -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: AgentInstanceId::new(),
        instance_revision_id: AgentInstanceRevisionId::new(),
        release_id: ReleaseId::new(),
        release_agent_id: ReleaseAgentId::new(),
        attachment_id: Some(AgentAttachmentId::new()),
        kind: RunKind::Normal,
        requires_state: true,
    }
}

pub fn scope() -> RunCleanupHostId {
    RunCleanupHostId::new("cleanup-root-owner-1".into(), "cleanup-host-1".into()).expect("scope")
}

pub async fn insert_run(pool: &PgPool, command: &StartRun) {
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'normal',$7,'queued',true,now(),now())")
        .bind(command.run_id.as_uuid()).bind(command.instance_id.as_uuid()).bind(command.instance_revision_id.as_uuid())
        .bind(command.release_id.as_uuid()).bind(command.release_agent_id.as_uuid()).bind(command.attachment_id.map(AgentAttachmentId::as_uuid))
        .bind(command.command_id.as_uuid()).execute(pool).await.expect("run");
}

pub struct Fixture {
    pub command: StartRun,
    pub volumes: [Uuid; 1],
    pub leases: [Uuid; 1],
}

impl Fixture {
    pub async fn seed(pool: &PgPool) -> Self {
        Self::seed_on_host(pool, "cleanup-host-1").await
    }

    pub async fn seed_on_host(pool: &PgPool, host: &str) -> Self {
        Self::seed_internal(pool, host, false).await
    }

    pub async fn seed_planned(pool: &PgPool, host: &str) -> Self {
        Self::seed_internal(pool, host, true).await
    }

    async fn seed_internal(pool: &PgPool, host: &str, planned: bool) -> Self {
        let command = command();
        super::support::seed_instance(pool, &command).await;
        if planned {
            create_planned(pool, &command).await;
        } else {
            insert_run(pool, &command).await;
        }
        let volumes = [Uuid::new_v4()];
        let leases = [Uuid::new_v4()];
        let origin = command.instance_id;
        let insert = if planned {
            sqlx::query(
                "INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,state,host_id,host_path,capacity_bytes,filesystem_uuid,lease_generation,provisioning_state) SELECT $1,id,project_id,'ready',$4,$2,16777216,gen_random_uuid(),6,'ready' FROM agent_instances WHERE id=$3",
            )
        } else {
            sqlx::query(
                "INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,state,host_id,host_path,capacity_bytes,filesystem_uuid,lease_generation,provisioning_state) SELECT $1,id,project_id,'attached',$4,$2,16777216,gen_random_uuid(),7,'ready' FROM agent_instances WHERE id=$3",
            )
        };
        insert
            .bind(volumes[0])
            .bind(format!("/private-cleanup-fixture/{}", volumes[0]))
            .bind(origin.as_uuid())
            .bind(host)
            .execute(pool)
            .await
            .expect("legacy resource");
        sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
            .bind(origin.as_uuid())
            .bind(volumes[0])
            .execute(pool)
            .await
            .expect("legacy pointer");
        if planned {
            insert_selected_lease(pool, &command, leases[0], volumes[0], host, 7).await;
        } else {
            sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,$5,7,'active',now()+interval '1 hour')")
                .bind(leases[0]).bind(volumes[0]).bind(origin.as_uuid()).bind(command.run_id.as_uuid()).bind(host).execute(pool).await.expect("historical canonical held lease");
        }
        Self {
            command,
            volumes,
            leases,
        }
    }

    pub async fn snapshot(&self, pool: &PgPool) -> Value {
        sqlx::query_scalar("SELECT jsonb_build_object('run',jsonb_build_array(id,instance_id,instance_revision_id,state,vm_id),'leases',(SELECT jsonb_agg(jsonb_build_array(id,volume_id,instance_id,run_id,host_id,fencing_token,state,released_at) ORDER BY id) FROM agent_instance_volume_leases WHERE run_id=$1),'volumes',(SELECT jsonb_agg(jsonb_build_array(id,instance_id,project_id,filesystem_uuid,host_path,capacity_bytes,lease_generation,state) ORDER BY id) FROM agent_instance_state_volumes WHERE id=ANY($2))) FROM runs WHERE id=$1")
            .bind(self.command.run_id.as_uuid()).bind(self.volumes.as_slice()).fetch_one(pool).await.expect("stable historical evidence")
    }
}

pub async fn create_planned(pool: &PgPool, command: &StartRun) {
    use run_orchestrator::RunRepository;
    let worker = role_pool(pool, Role::Worker).await;
    run_postgres::PgRunRepository::new(worker.clone())
        .with_vm_owner_scope(scope())
        .create_run_with_vm_plan(
            command,
            &scope(),
            &vm_trait::VmId(command.run_id.to_string()),
        )
        .await
        .expect("atomic fresh run ownership");
    worker.close().await;
}

pub async fn insert_selected_lease(
    pool: &PgPool,
    command: &StartRun,
    lease: Uuid,
    volume: Uuid,
    host: &str,
    fence: i64,
) {
    sqlx::query("UPDATE runs SET state='leasing_volume' WHERE id=$1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at,selection_origin,selected_revision_id,selected_release_id,selected_release_agent_id,selected_project_id,slot_key,access_mode,guest_path,slot_required,minimum_capacity_bytes,release_contract_hash) SELECT $1,$2,execution.instance_id,execution.id,$4,$5,'active',now()+interval '1 hour','legacy_origin',execution.instance_revision_id,execution.release_id,execution.release_agent_id,consumer.project_id,'state','read_write','/var/lib/hephaestus',true,1,agent.runtime_contract_hash FROM runs execution JOIN agent_instances consumer ON consumer.id=execution.instance_id JOIN release_agents agent ON agent.id=execution.release_agent_id WHERE execution.id=$3")
        .bind(lease).bind(volume).bind(command.run_id.as_uuid()).bind(host).bind(fence).execute(pool).await.expect("exact legacy lease selection");
    sqlx::query("UPDATE agent_instance_state_volumes SET state='attached' WHERE id=$1 AND lease_generation=$2")
        .bind(volume).bind(fence).execute(pool).await.expect("fixture models attached exact fence");
}

pub async fn checksums(pool: &PgPool) -> Vec<(i64, Vec<u8>)> {
    sqlx::query_as(
        "SELECT version,checksum FROM _sqlx_migrations WHERE version<=107 ORDER BY version",
    )
    .fetch_all(pool)
    .await
    .expect("historical migration checksums")
}

#[derive(Clone, Copy)]
pub enum Role {
    Actor,
    Worker,
}

pub async fn role_pool(pool: &PgPool, role: Role) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
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
        .expect("actual role pool")
}
