use std::borrow::Cow;

use sqlx::{
    PgPool, Postgres, Transaction,
    migrate::{Migration, MigrationType, Migrator},
};
use uuid::Uuid;

use crate::support::Fixture;

const PUBLIC: Migrator = sqlx::migrate!("../../../../../migrations");

pub fn checkpoint(version: i64) -> Migrator {
    let mut migrations = PUBLIC
        .iter()
        .filter(|m| m.version <= version.min(107))
        .cloned()
        .collect::<Vec<_>>();
    if version == 108 {
        assert_eq!(
            std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
            Ok("1")
        );
        let sql = [
            include_str!("../../../../../../../migrations/0108_run_volume_leases.sql"),
            include_str!("../../../../../../../migrations/0109_run_cleanup.sql"),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .join("\n");
        migrations.push(Migration::new(
            108,
            Cow::Borrowed("private_profile_foundation"),
            MigrationType::Simple,
            Cow::Owned(sql),
            false,
        ));
    }
    Migrator {
        migrations: Cow::Owned(migrations),
        ..Migrator::DEFAULT
    }
}

pub async fn baseline(pool: &PgPool) -> Fixture {
    checkpoint(100).run(pool).await.unwrap();
    let f = crate::seed::seed(pool).await;
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(f.instance)
        .bind(f.volume)
        .execute(pool)
        .await
        .unwrap();
    checkpoint(107).run(pool).await.unwrap();
    f
}

pub async fn setup(pool: &PgPool) -> Fixture {
    let f = baseline(pool).await;
    checkpoint(108).run(pool).await.unwrap();
    f
}

#[derive(Clone)]
pub struct Execution {
    pub run: Uuid,
    pub command: Uuid,
    pub instance: Uuid,
    pub revision: Uuid,
    pub agent: Uuid,
    pub release: Uuid,
    pub attachment: Option<Uuid>,
    pub requires_state: bool,
    pub kind: &'static str,
}

impl Execution {
    pub async fn normal(pool: &PgPool, f: &Fixture) -> Self {
        Self {
            run: Uuid::new_v4(),
            command: Uuid::new_v4(),
            instance: f.instance,
            revision: sqlx::query_scalar(
                "SELECT active_revision_id FROM agent_instances WHERE id=$1",
            )
            .bind(f.instance)
            .fetch_one(pool)
            .await
            .unwrap(),
            agent: f.release_agent,
            release: f.release,
            attachment: Some(f.attachment),
            requires_state: true,
            kind: "normal",
        }
    }

    pub async fn stateless(pool: &PgPool, f: &Fixture) -> Self {
        let mut e = Self::normal(pool, f).await;
        let agent = Uuid::new_v4();
        let revision = Uuid::new_v4();
        sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,release_id,family_id,$2,'profile stateless',jsonb_build_object('requires_state',false),$3,false FROM release_agents WHERE id=$4")
            .bind(agent).bind(format!("profile-{agent}")).bind([42_u8;32].as_slice()).bind(e.agent).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,$2,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=$3")
            .bind(revision).bind(agent).bind(e.revision).execute(pool).await.unwrap();
        sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
            .bind(e.instance)
            .bind(revision)
            .execute(pool)
            .await
            .unwrap();
        e.revision = revision;
        e.agent = agent;
        e.requires_state = false;
        e
    }

    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({"run_id":self.run,"command_id":self.command,"instance_id":self.instance,"instance_revision_id":self.revision,"release_id":self.release,"release_agent_id":self.agent,"attachment_id":self.attachment,"kind":self.kind,"requires_state":self.requires_state})
    }
}

pub async fn worker(pool: &PgPool) -> Transaction<'_, Postgres> {
    let mut tx = pool.begin().await.unwrap();
    sqlx::query("SET LOCAL ROLE hephaestus_worker")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx
}

pub async fn insert(
    tx: &mut Transaction<'_, Postgres>,
    e: &Execution,
    profile: &str,
    scope: bool,
    admission: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO runs(id,command_id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,requires_state,state,created_at,updated_at,execution_profile,vm_id,vm_provider_namespace,vm_host_id,vm_binding_kind,vm_plan_admission_required) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,'queued',now(),now(),$10,$11,$12,$13,$14,$15)")
        .bind(e.run).bind(e.command).bind(e.instance).bind(e.revision).bind(e.release).bind(e.agent).bind(e.attachment).bind(e.kind).bind(e.requires_state).bind(profile)
        .bind(scope.then(||e.run.to_string())).bind(scope.then_some("profile-provider")).bind(scope.then_some("host")).bind(scope.then_some("planned")).bind(admission)
        .execute(&mut **tx).await.map(|_|())
}

pub async fn snapshot(pool: &PgPool, run: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT to_jsonb(r)-'execution_profile'-'volume_acquisition_closed_at'-'volume_selection_snapshot'-'vm_plan_admission_required'-'vm_provider_namespace'-'vm_host_id'-'vm_binding_kind' FROM runs r WHERE id=$1")
        .bind(run).fetch_one(pool).await.unwrap()
}

#[derive(Clone, Copy)]
pub enum Mutation {
    Progress,
    Bind,
    Clean,
    Promote,
    Pin,
    Scope,
    Attached,
    Heartbeat,
    Release,
}

pub async fn denied(pool: &PgPool, run: Uuid, mutation: Mutation) {
    let before = snapshot(pool, run).await;
    let mut tx = worker(pool).await;
    let query = match mutation {
        Mutation::Progress => sqlx::query("UPDATE runs SET state='provisioning' WHERE id=$1"),
        Mutation::Bind => sqlx::query("UPDATE runs SET vm_id=id::text WHERE id=$1"),
        Mutation::Clean => sqlx::query("UPDATE runs SET state='cleaned_up' WHERE id=$1"),
        Mutation::Promote => {
            sqlx::query("UPDATE runs SET execution_profile='owned_canonical' WHERE id=$1")
        }
        Mutation::Pin => sqlx::query("UPDATE runs SET command_id=gen_random_uuid() WHERE id=$1"),
        Mutation::Scope => sqlx::query(
            "UPDATE runs SET vm_provider_namespace='profile-provider',vm_host_id='host',vm_binding_kind='planned',vm_id=id::text WHERE id=$1",
        ),
        Mutation::Attached => {
            sqlx::query("UPDATE agent_instance_volume_leases SET attached_at=now() WHERE run_id=$1")
        }
        Mutation::Heartbeat => sqlx::query(
            "UPDATE agent_instance_volume_leases SET heartbeat_at=now()+interval '1 hour' WHERE run_id=$1",
        ),
        Mutation::Release => sqlx::query(
            "UPDATE agent_instance_volume_leases SET released_at=now(),state='released' WHERE run_id=$1",
        ),
    };
    let error = query
        .bind(run)
        .execute(&mut *tx)
        .await
        .expect_err("profile guard must reject mutation");
    assert_eq!(
        error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .as_deref(),
        Some("23000")
    );
    tx.rollback().await.unwrap();
    assert_eq!(snapshot(pool, run).await, before);
}

pub async fn advance(pool: &PgPool, run: Uuid, state: &str) {
    let mut tx = worker(pool).await;
    sqlx::query("UPDATE runs SET state=$2 WHERE id=$1")
        .bind(run)
        .bind(state)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

pub async fn lease(pool: &PgPool, e: &Execution, f: &Fixture) -> Result<Uuid, sqlx::Error> {
    let id = Uuid::new_v4();
    let mut tx = worker(pool).await;
    sqlx::query("INSERT INTO agent_instance_volume_leases(id,volume_id,instance_id,run_id,host_id,fencing_token,state,expires_at) VALUES($1,$2,$3,$4,'host',1,'active',now()+interval '1 hour')")
        .bind(id).bind(f.volume).bind(e.instance).bind(e.run).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}
