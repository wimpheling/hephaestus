use std::{borrow::Cow, sync::Arc};

use authz_postgres::PostgresMelangeAuthorizer;
use release_postgres::ReleaseService;
use run_domain::{FreshRunExecutionProfile, LegacyVmPlacementScope, RunKind, StartRun};
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use uuid::Uuid;
use vm_trait::VmProviderOwnerScope;

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

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
        requires_state: false,
    }
}

pub fn owner() -> LegacyVmPlacementScope {
    LegacyVmPlacementScope::new(
        VmProviderOwnerScope::new(Uuid::new_v4().to_string(), "release-observer-host".into())
            .unwrap(),
    )
    .unwrap()
}

pub async fn schema(pool: &PgPool) {
    assert_eq!(
        std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
        Ok("1")
    );
    public_schema(pool).await;
    for sql in [
        include_str!("/var/tmp/0108_run_volume_leases.profile.sql"),
        include_str!("/var/tmp/0108_run_cleanup_fragment.profile.sql"),
        include_str!("/var/tmp/0108_legacy_scalar_compatibility.sql"),
        include_str!("/var/tmp/0108_run_profile_lifecycle.sql"),
        include_str!("/var/tmp/0108_legacy_vm_placement.sql"),
    ] {
        sqlx::raw_sql(sql).execute(pool).await.unwrap();
    }
}

/// Installs only public107 so historical fixtures precede private birth guards.
pub async fn public_schema(pool: &PgPool) {
    Migrator {
        migrations: Cow::Owned(
            MIGRATIONS
                .iter()
                .filter(|m| m.version <= 107)
                .cloned()
                .collect(),
        ),
        ..Migrator::DEFAULT
    }
    .run(pool)
    .await
    .unwrap();
}

pub async fn role(pool: &PgPool, worker: bool) -> PgPool {
    PgPoolOptions::new()
        .max_connections(3)
        .after_connect(move |connection, _| {
            Box::pin(async move {
                if worker {
                    sqlx::query("SET ROLE hephaestus_worker")
                        .execute(connection)
                        .await?;
                } else {
                    sqlx::query("SET ROLE hephaestus_app")
                        .execute(connection)
                        .await?;
                }
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap()
}

pub fn service(pool: &PgPool, scope: &LegacyVmPlacementScope) -> ReleaseService {
    ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer))
        .with_execution_profile(FreshRunExecutionProfile::LegacyScalar)
        .with_legacy_vm_placement(scope.clone())
}

pub async fn fresh(
    pool: &PgPool,
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
    stateful: bool,
    mailbox: bool,
) -> StartRun {
    let mut run = command();
    run.requires_state = stateful;
    super::support::seed_instance(pool, &run).await;
    birth(pool, worker, scope, &run, mailbox).await;
    run
}

pub async fn birth(
    pool: &PgPool,
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
    run: &StartRun,
    mailbox: bool,
) {
    if !mailbox {
        super::support::seed_run_request(pool, run).await;
    }
    let mut tx = worker.begin().await.unwrap();
    sqlx::query("SELECT id FROM agent_instances WHERE id=$1 FOR UPDATE")
        .bind(run.instance_id.as_uuid())
        .fetch_one(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO command_inbox(command_id,command_type,payload) VALUES($1,'start_run',$2)",
    )
    .bind(run.command_id.as_uuid())
    .bind(serde_json::to_value(run).unwrap())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,command_id,run_kind,state,requires_state,created_at,updated_at,execution_profile,legacy_vm_placement_required) VALUES($1,$2,$3,$4,$5,$6,$7,'normal','queued',$8,now(),now(),'legacy_scalar',true)")
        .bind(run.run_id.as_uuid()).bind(run.instance_id.as_uuid()).bind(run.instance_revision_id.as_uuid()).bind(run.release_id.as_uuid()).bind(run.release_agent_id.as_uuid()).bind(run.attachment_id.unwrap().as_uuid()).bind(run.command_id.as_uuid()).bind(run.requires_state).execute(&mut *tx).await.unwrap();
    if mailbox {
        // The immediate attempt FK needs its Run first, while the deferred
        // positive profile/placement proof commits the whole producer together.
        mailbox_input(&mut tx, run).await;
    }
    sqlx::query("SELECT record_legacy_vm_placement($1,$2,$3,$4)")
        .bind(run.run_id.as_uuid())
        .bind(namespace(scope))
        .bind(scope.provider_scope().host_id())
        .bind(if mailbox { "mailbox" } else { "normal_request" })
        .execute(&mut *tx)
        .await
        .unwrap();
    if !mailbox {
        // Match production fresh Normal birth: a delivered command has already
        // left the pending request queue before any deferred update can drain.
        let delivered = sqlx::query("UPDATE run_requests SET dispatch_state='dispatched' WHERE run_id=$1 AND command_id=$2 AND dispatch_state='pending'")
            .bind(run.run_id.as_uuid()).bind(run.command_id.as_uuid()).execute(&mut *tx).await.unwrap();
        assert_eq!(delivered.rows_affected(), 1);
    }
    tx.commit().await.unwrap();
    if mailbox {
        consume(worker, scope, run).await;
    }
}

async fn mailbox_input(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>, run: &StartRun) {
    let project: Uuid = sqlx::query_scalar("SELECT project_id FROM agent_instances WHERE id=$1")
        .bind(run.instance_id.as_uuid())
        .fetch_one(&mut **tx)
        .await
        .unwrap();
    let mailbox = Uuid::new_v4();
    let payload = Uuid::new_v4();
    let event = Uuid::new_v4();
    sqlx::query("INSERT INTO mailboxes(id,project_id,instance_id,state) VALUES($1,$2,$3,'active')")
        .bind(mailbox)
        .bind(project)
        .bind(run.instance_id.as_uuid())
        .execute(&mut **tx)
        .await
        .unwrap();
    let body = b"observer fixture".as_slice();
    sqlx::query("INSERT INTO mailbox_payloads(id,mailbox_id,project_id,encoded_body,encoded_length,decoded_length,integrity_hash) VALUES($1,$2,$3,$4,octet_length($4),octet_length($4),sha256($4))")
        .bind(payload).bind(mailbox).bind(project).bind(body).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO mailbox_events(id,mailbox_id,project_id,instance_id,body_id,producer_kind,producer_id,deduplication_scope,deduplication_key,method,route,received_at) VALUES($1,$2,$3,$4,$5,'gateway','observer','observer',$6,'POST','/observer',now())")
        .bind(event).bind(mailbox).bind(project).bind(run.instance_id.as_uuid()).bind(payload)
        .bind(format!("event-{event}")).execute(&mut **tx).await.unwrap();
    sqlx::query("INSERT INTO mailbox_delivery_attempts(id,event_id,mailbox_id,attempt_number,state,command_id,instance_id,instance_revision_id,run_id,target_ref,target_commit,state_access_outcome) VALUES($1,$2,$3,1,'leased',$1,$4,$5,$6,'refs/heads/main',$7,'no_state')")
        .bind(run.command_id.as_uuid()).bind(event).bind(mailbox).bind(run.instance_id.as_uuid())
        .bind(run.instance_revision_id.as_uuid()).bind(run.run_id.as_uuid()).bind("a".repeat(40))
        .execute(&mut **tx).await.unwrap();
}

pub fn namespace(scope: &LegacyVmPlacementScope) -> Uuid {
    scope.provider_scope().namespace().parse().unwrap()
}

pub async fn consume(worker: &PgPool, scope: &LegacyVmPlacementScope, run: &StartRun) {
    sqlx::query("SELECT consume_legacy_vm_placement($1,$2,$3,$4,'provision')")
        .bind(run.run_id.as_uuid())
        .bind(run.command_id.as_uuid())
        .bind(namespace(scope))
        .bind(scope.provider_scope().host_id())
        .execute(worker)
        .await
        .unwrap();
}

pub async fn transition(worker: &PgPool, run: &StartRun, state: &str) {
    sqlx::query("UPDATE runs SET state=$2,state_version=state_version+1 WHERE id=$1")
        .bind(run.run_id.as_uuid())
        .bind(state)
        .execute(worker)
        .await
        .unwrap();
}

pub async fn running(worker: &PgPool, run: &StartRun) {
    transition(worker, run, "leasing_volume").await;
    sqlx::query("UPDATE runs SET vm_id=$2 WHERE id=$1")
        .bind(run.run_id.as_uuid())
        .bind(run.run_id.to_string())
        .execute(worker)
        .await
        .unwrap();
    for state in ["provisioning", "starting", "running"] {
        transition(worker, run, state).await;
    }
}

pub async fn close(worker: &PgPool, scope: &LegacyVmPlacementScope, run: &StartRun) {
    sqlx::query("SELECT close_legacy_vm_acquisition($1,$2,$3)")
        .bind(run.run_id.as_uuid())
        .bind(namespace(scope))
        .bind(scope.provider_scope().host_id())
        .execute(worker)
        .await
        .unwrap();
}

pub async fn outcome(worker: &PgPool, run: &StartRun, success: bool) {
    sqlx::query("UPDATE runs SET state=$2,outcome=$3,exit_code=$4,state_version=state_version+1 WHERE id=$1")
        .bind(run.run_id.as_uuid()).bind(if success { "succeeded" } else { "failed" }).bind(if success { "succeeded" } else { "failed" }).bind(if success { 0 } else { 17 }).execute(worker).await.unwrap();
    transition(worker, run, "cleaning_up").await;
}

pub async fn finish(worker: &PgPool, scope: &LegacyVmPlacementScope, run: &StartRun) {
    close(worker, scope, run).await;
    outcome(worker, run, true).await;
    // Controlled confirmation boundary: caller supplies the trusted Core's
    // transition order. This fixture does not claim physical VM destruction.
    transition(worker, run, "cleaned_up").await;
}

pub async fn snapshot(pool: &PgPool, run: &StartRun) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',to_jsonb(execution),'instance',(SELECT to_jsonb(consumer) FROM agent_instances consumer WHERE consumer.id=execution.instance_id),'leases',(SELECT COALESCE(jsonb_agg(to_jsonb(lease) ORDER BY lease.id),'[]'::jsonb) FROM agent_instance_volume_leases lease WHERE lease.instance_id=execution.instance_id),'volumes',(SELECT COALESCE(jsonb_agg(to_jsonb(volume) ORDER BY volume.id),'[]'::jsonb) FROM agent_instance_state_volumes volume WHERE volume.instance_id=execution.instance_id),'updates',(SELECT COALESCE(jsonb_agg(to_jsonb(candidate) ORDER BY candidate.id),'[]'::jsonb) FROM agent_updates candidate WHERE candidate.instance_id=execution.instance_id)) FROM runs execution WHERE execution.id=$1").bind(run.run_id.as_uuid()).fetch_one(pool).await.unwrap()
}

#[derive(sqlx::FromRow)]
pub struct Lease {
    pub id: Uuid,
    pub volume_id: Uuid,
    pub fencing_token: i64,
}

pub async fn acquire(
    pool: &PgPool,
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
    run: &StartRun,
    existing: Option<Uuid>,
) -> Lease {
    let volume = if let Some(volume) = existing {
        volume
    } else {
        let volume = Uuid::new_v4();
        sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,state,host_id,host_path,capacity_bytes,filesystem_uuid,provisioning_state) SELECT $1,id,project_id,'ready',$2,$3,16777216,gen_random_uuid(),'ready' FROM agent_instances WHERE id=$4")
            .bind(volume).bind(scope.provider_scope().host_id()).bind(format!("/controlled-release/{volume}")).bind(run.instance_id.as_uuid()).execute(pool).await.unwrap();
        sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
            .bind(run.instance_id.as_uuid())
            .bind(volume)
            .execute(pool)
            .await
            .unwrap();
        volume
    };
    transition(worker, run, "leasing_volume").await;
    sqlx::query_as("SELECT id,volume_id,fencing_token FROM legacy_scalar_acquire($1,$2,$3,clock_timestamp(),clock_timestamp()+interval '5 minutes')")
        .bind(volume).bind(run.run_id.as_uuid()).bind(scope.provider_scope().host_id()).fetch_one(worker).await.unwrap()
}

pub async fn detach(
    worker: &PgPool,
    scope: &LegacyVmPlacementScope,
    run: &StartRun,
    lease: &Lease,
) {
    sqlx::query("SELECT release_legacy_scalar_lease_after_detach($1,$2,$3,$4,$5,false)")
        .bind(lease.id)
        .bind(lease.volume_id)
        .bind(run.run_id.as_uuid())
        .bind(scope.provider_scope().host_id())
        .bind(lease.fencing_token)
        .execute(worker)
        .await
        .unwrap();
}
