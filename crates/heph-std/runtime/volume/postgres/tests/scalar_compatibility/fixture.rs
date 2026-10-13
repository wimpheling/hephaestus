use std::borrow::Cow;

use runtime_types::{RunId, VolumeId};
use sqlx::{
    PgPool, Postgres, Transaction,
    migrate::{Migration, MigrationType, Migrator},
    postgres::PgPoolOptions,
};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use volume_trait::{VolumeLease, VolumeMetadataRepository};

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
            include_str!("../../../../../../../migrations/0110_legacy_scalar_compatibility.sql"),
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>()
        .join("\n");
        migrations.push(Migration::new(
            108,
            Cow::Borrowed("private_scalar_compatibility"),
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
    sqlx::query("UPDATE agent_instance_state_volumes SET provisioning_state='ready' WHERE id=$1")
        .bind(f.volume)
        .execute(pool)
        .await
        .unwrap();
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

    pub fn payload(&self) -> serde_json::Value {
        serde_json::json!({"run_id":self.run,"command_id":self.command,"instance_id":self.instance,"instance_revision_id":self.revision,"release_id":self.release,"release_agent_id":self.agent,"attachment_id":self.attachment,"kind":self.kind,"requires_state":self.requires_state})
    }

    pub async fn stateless(pool: &PgPool, f: &Fixture) -> Self {
        let mut execution = Self::normal(pool, f).await;
        let agent = Uuid::new_v4();
        let revision = Uuid::new_v4();
        sqlx::query("INSERT INTO release_agents(id,release_id,family_id,agent_key,display_name,runtime_contract,runtime_contract_hash,requires_state) SELECT $1,release_id,family_id,$2,'scalar stateless',jsonb_build_object('requires_state',false),$3,false FROM release_agents WHERE id=$4")
            .bind(agent).bind(format!("scalar-{agent}")).bind([42_u8;32].as_slice()).bind(execution.agent).execute(pool).await.unwrap();
        sqlx::query("INSERT INTO agent_instance_revisions(id,instance_id,release_agent_id,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,runnable) SELECT $1,instance_id,$2,parameters,parameter_hash,resource_selection,network_restriction,effective_runtime_policy,effective_policy_hash,platform_policy_version,true FROM agent_instance_revisions WHERE id=$3")
            .bind(revision).bind(agent).bind(execution.revision).execute(pool).await.unwrap();
        sqlx::query("UPDATE agent_instances SET active_revision_id=$2 WHERE id=$1")
            .bind(execution.instance)
            .bind(revision)
            .execute(pool)
            .await
            .unwrap();
        execution.revision = revision;
        execution.agent = agent;
        execution.requires_state = false;
        execution
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

pub async fn repository(pool: &PgPool) -> PostgresVolumeMetadataRepository {
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .after_connect(|connection, _| {
            Box::pin(async move {
                sqlx::query("SET ROLE hephaestus_worker")
                    .execute(connection)
                    .await?;
                Ok(())
            })
        })
        .connect_with(pool.connect_options().as_ref().clone())
        .await
        .unwrap();
    PostgresVolumeMetadataRepository::new(pool).with_scalar_schema(ScalarMetadataSchema::Profiled)
}

pub fn times() -> (OffsetDateTime, OffsetDateTime) {
    let now = OffsetDateTime::now_utc();
    (now, now + time::Duration::minutes(5))
}

pub async fn acquired(
    pool: &PgPool,
) -> (
    Fixture,
    Execution,
    PostgresVolumeMetadataRepository,
    VolumeLease,
) {
    acquired_with_ttl(pool, time::Duration::minutes(5)).await
}

pub async fn acquired_with_ttl(
    pool: &PgPool,
    ttl: time::Duration,
) -> (
    Fixture,
    Execution,
    PostgresVolumeMetadataRepository,
    VolumeLease,
) {
    let f = setup(pool).await;
    let e = Execution::normal(pool, &f).await;
    super::producers::normal(pool, &e, "legacy_scalar", false)
        .await
        .unwrap();
    advance(pool, e.run, "leasing_volume").await;
    let repository = repository(pool).await;
    let now = OffsetDateTime::now_utc();
    let expires = now + ttl;
    let lease = repository
        .acquire(
            VolumeId::from_uuid(f.volume),
            RunId::from_uuid(e.run),
            "host",
            now,
            expires,
        )
        .await
        .unwrap();
    (f, e, repository, lease)
}

pub async fn evidence(pool: &PgPool, volume: Uuid) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('volume',to_jsonb(volume),'leases',COALESCE((SELECT jsonb_agg(to_jsonb(lease) ORDER BY lease.id) FROM agent_instance_volume_leases lease WHERE lease.volume_id=volume.id),'[]'::jsonb)) FROM agent_instance_state_volumes volume WHERE volume.id=$1")
        .bind(volume).fetch_one(pool).await.unwrap()
}

pub async fn expire(pool: &PgPool, lease: &VolumeLease) {
    // Let actual time pass; never rewrite immutable acquisition or manufacture
    // an expired selection by changing raw timestamps after admission.
    let delay = lease.expires_at - OffsetDateTime::now_utc();
    if delay.is_positive() {
        assert!(
            delay <= time::Duration::seconds(3),
            "only short expiry fixtures may wait"
        );
        tokio::time::sleep(std::time::Duration::try_from(delay).unwrap()).await;
    }
    let expired: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_instance_volume_leases WHERE id=$1 AND expires_at<=clock_timestamp()")
        .bind(lease.id.as_uuid()).fetch_one(pool).await.unwrap();
    assert_eq!(expired, 1);
}
