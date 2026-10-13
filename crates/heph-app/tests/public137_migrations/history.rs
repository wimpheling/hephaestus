//! Public107 data only. This fixture proves historical inputs, not admission.
use release_domain::{AgentInstanceId, AgentInstanceRevisionId};
use runtime_types::RunId;
use serde_json::Value;
use sqlx::PgPool;
use std::collections::BTreeMap;
use uuid::Uuid;

pub type Rows = BTreeMap<&'static str, Vec<Value>>;

pub struct InstanceHistory {
    pub instance: AgentInstanceId,
    pub revision: AgentInstanceRevisionId,
    pub attachment: Uuid,
    pub normal: RunId,
    pub release: Uuid,
    pub agent: Uuid,
}

pub struct LeaseHistory {
    pub id: Uuid,
    pub volume: Uuid,
    pub run: RunId,
    pub instance: AgentInstanceId,
    pub host: &'static str,
    pub fence: i64,
}

#[derive(sqlx::FromRow)]
pub struct LegacySelection {
    pub lease: Uuid,
    pub instance: Uuid,
    pub volume: Uuid,
    pub revision: Uuid,
    pub release: Uuid,
    pub agent: Uuid,
    pub project: Uuid,
    pub contract_hash: Vec<u8>,
}

pub struct History {
    pub qualifying_instance: InstanceHistory,
    pub unknown_instance: InstanceHistory,
    pub update: RunId,
    pub qualifying_lease: LeaseHistory,
    pub unknown_lease: LeaseHistory,
    pub qualifying_before: Vec<LegacySelection>,
    pub before: Rows,
}

pub async fn seed(pool: &PgPool) -> History {
    // Separate support seeds give each graph its own repository/release namespace.
    let qualifying_source = crate::support::seed(pool).await;
    let unknown_source = crate::support::seed(pool).await;
    let qualifying_instance = instance(pool, &qualifying_source).await;
    let unknown_instance = instance(pool, &unknown_source).await;
    // Public107 permits this independent legacy flag; retain the original hash.
    sqlx::query("UPDATE release_agents SET requires_state=true WHERE id=(SELECT release_agent_id FROM runs WHERE id=$1)")
        .bind(qualifying_instance.normal.as_uuid()).execute(pool).await
        .expect("qualifying public107 legacy state declaration");
    let update = update(pool, &unknown_instance).await;
    let qualifying_lease = lease(
        pool,
        &qualifying_instance,
        qualifying_instance.normal,
        "upgrade-qualifying-host",
    )
    .await;
    let unknown_lease = lease(pool, &unknown_instance, update, "upgrade-unknown-host").await;
    let qualifying_before = qualifying(pool).await;
    assert_eq!(qualifying_before.len(), 1);
    assert_eq!(qualifying_before[0].lease, qualifying_lease.id);
    assert!(
        qualifying_before
            .iter()
            .all(|selection| selection.lease != unknown_lease.id)
    );
    History {
        qualifying_instance,
        unknown_instance,
        update,
        qualifying_lease,
        unknown_lease,
        qualifying_before,
        before: rows(pool).await,
    }
}

async fn instance(pool: &PgPool, source: &crate::support::Fixture) -> InstanceHistory {
    let (instance, revision, attachment, normal) =
        crate::support::seed_instance(pool, source).await;
    let (release, agent) = sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT release_id,release_agent_id FROM runs WHERE id=$1",
    )
    .bind(normal.as_uuid())
    .fetch_one(pool)
    .await
    .expect("original run release identities");
    InstanceHistory {
        instance,
        revision,
        attachment,
        normal,
        release,
        agent,
    }
}

async fn update(pool: &PgPool, instance: &InstanceHistory) -> RunId {
    let run = RunId::new();
    sqlx::query("INSERT INTO runs (id,instance_id,instance_revision_id,release_id,release_agent_id,run_kind,command_id,state,requires_state,created_at,updated_at) SELECT $1,instance_id,instance_revision_id,release_id,release_agent_id,'update',$2,'queued',false,now(),now() FROM runs WHERE id=$3 AND instance_id=$4 AND instance_revision_id=$5")
        .bind(run.as_uuid()).bind(Uuid::new_v4()).bind(instance.normal.as_uuid())
        .bind(instance.instance.as_uuid()).bind(instance.revision.as_uuid())
        .execute(pool).await.expect("ordinary public107 Update relationships");
    run
}

async fn lease(
    pool: &PgPool,
    instance: &InstanceHistory,
    run: RunId,
    host: &'static str,
) -> LeaseHistory {
    let volume = Uuid::new_v4();
    let id = Uuid::new_v4();
    sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,state,host_id,host_path,capacity_bytes,filesystem_uuid,lease_generation,provisioning_state) SELECT $1,id,project_id,'attached',$4,$2,16777216,gen_random_uuid(),7,'ready' FROM agent_instances WHERE id=$3")
        .bind(volume).bind(format!("public137-history/{volume}"))
        .bind(instance.instance.as_uuid()).bind(host).execute(pool).await
        .expect("public107 attached volume with original generation");
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(instance.instance.as_uuid())
        .bind(volume)
        .execute(pool)
        .await
        .expect("legacy pointer");
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,$5,7,'active',now()+interval '1 hour')")
        .bind(id).bind(volume).bind(instance.instance.as_uuid()).bind(run.as_uuid())
        .bind(host).execute(pool).await.expect("public107 held lease");
    LeaseHistory {
        id,
        volume,
        run,
        instance: instance.instance,
        host,
        fence: 7,
    }
}

pub async fn qualifying(pool: &PgPool) -> Vec<LegacySelection> {
    sqlx::query_as(
        "SELECT lease.id AS lease,instance.id AS instance,volume.id AS volume,
            run.instance_revision_id AS revision,run.release_id AS release,
            run.release_agent_id AS agent,volume.project_id AS project,
            agent.runtime_contract_hash AS contract_hash
        FROM agent_instance_volume_leases lease
        JOIN runs run ON lease.run_id=run.id
        JOIN agent_instances instance ON instance.id=run.instance_id
        JOIN agent_instance_revisions revision ON revision.id=run.instance_revision_id
            AND revision.instance_id=instance.id
            AND revision.release_agent_id=run.release_agent_id
        JOIN release_agents agent ON agent.id=run.release_agent_id AND agent.release_id=run.release_id
        JOIN agent_instance_state_volumes volume ON volume.id=instance.state_volume_id
            AND volume.instance_id=instance.id AND volume.project_id=instance.project_id
        WHERE lease.instance_id=instance.id AND lease.volume_id=volume.id
            AND instance.volume_mode='legacy' AND agent.requires_state
            AND octet_length(agent.runtime_contract_hash)=32
            AND jsonb_array_length(COALESCE(agent.runtime_contract->'volume_slots','[]'::jsonb))=0
        ORDER BY lease.id",
    )
    .fetch_all(pool)
    .await
    .expect("independent pre108 frozen legacy qualification")
}

pub const TABLES: [&str; 31] = [
    "_sqlx_migrations",
    "melange_migrations",
    "melange_tuples",
    "users",
    "organizations",
    "organization_members",
    "organization_secret_managers",
    "projects",
    "project_secret_roles",
    "project_maintainers",
    "repositories",
    "build_requests",
    "agent_families",
    "releases",
    "release_agents",
    "release_artifacts",
    "agent_instances",
    "agent_instance_revisions",
    "agent_attachments",
    "runs",
    "run_events",
    "agent_updates",
    "agent_instance_events",
    "agent_instance_state_volumes",
    "agent_instance_volume_leases",
    "agent_instance_revision_volume_bindings",
    "application_event_scopes",
    "application_aggregate_versions",
    "application_events",
    "product_event_outbox",
    "product_event_dead_letters",
];

pub async fn rows(pool: &PgPool) -> Rows {
    let mut result = BTreeMap::new();
    for table in TABLES {
        let query = match table {
            "melange_migrations" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM melange_migrations t")
            }
            "melange_tuples" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM melange_tuples t")
            }
            "_sqlx_migrations" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM _sqlx_migrations t")
            }
            "users" => sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM users t"),
            "organizations" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM organizations t")
            }
            "organization_members" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM organization_members t")
            }
            "organization_secret_managers" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM organization_secret_managers t",
            ),
            "projects" => sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM projects t"),
            "project_secret_roles" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM project_secret_roles t")
            }
            "project_maintainers" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM project_maintainers t")
            }
            "repositories" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM repositories t")
            }
            "build_requests" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM build_requests t")
            }
            "agent_families" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_families t")
            }
            "releases" => sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM releases t"),
            "release_agents" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM release_agents t")
            }
            "release_artifacts" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM release_artifacts t")
            }
            "agent_instances" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_instances t")
            }
            "agent_instance_revisions" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_instance_revisions t")
            }
            "agent_attachments" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_attachments t")
            }
            "runs" => sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM runs t"),
            "run_events" => sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM run_events t"),
            "agent_updates" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_updates t")
            }
            "agent_instance_events" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM agent_instance_events t")
            }
            "agent_instance_state_volumes" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM agent_instance_state_volumes t",
            ),
            "agent_instance_volume_leases" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM agent_instance_volume_leases t",
            ),
            "agent_instance_revision_volume_bindings" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM agent_instance_revision_volume_bindings t",
            ),
            "application_event_scopes" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM application_event_scopes t")
            }
            "application_aggregate_versions" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM application_aggregate_versions t",
            ),
            "application_events" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM application_events t")
            }
            "product_event_outbox" => {
                sqlx::query_scalar::<_, Value>("SELECT to_jsonb(t) FROM product_event_outbox t")
            }
            "product_event_dead_letters" => sqlx::query_scalar::<_, Value>(
                "SELECT to_jsonb(t) FROM product_event_dead_letters t",
            ),
            _ => unreachable!("closed literal table inventory"),
        };
        let mut values = query
            .fetch_all(pool)
            .await
            .expect("complete original public107 rows");
        values.sort_by_key(Value::to_string);
        result.insert(table, values);
    }
    result
}
