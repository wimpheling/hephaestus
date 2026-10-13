use std::{borrow::Cow, sync::Arc};

use runtime_types::RunId;
use sqlx::{PgPool, migrate::Migrator, postgres::PgPoolOptions};
use time::OffsetDateTime;
use uuid::Uuid;
use volume_postgres::PostgresVolumeMetadataRepository;

use crate::{seed, support::Fixture};

const MIGRATIONS: Migrator = sqlx::migrate!("../../../../../migrations");

pub async fn baseline(pool: &PgPool) -> Fixture {
    checkpoint(100).run(pool).await.expect("legacy schema100");
    let fixture = seed::seed(pool).await;
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(fixture.instance)
        .bind(fixture.volume)
        .execute(pool)
        .await
        .expect("real legacy origin pointer");
    checkpoint(107)
        .run(pool)
        .await
        .expect("current published schema107");
    fixture
}

pub async fn apply_private(pool: &PgPool) {
    if std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref() == Ok("1") {
        crate::private_migrations::checkpoint(111)
            .run(pool)
            .await
            .expect("ordered private111 lease/provisioning regression schema");
        return;
    }
    // This uncommitted staging harness reads only the two task-authored private
    // migrations. It will become ordinary Migrator execution at publication.
    for variable in [
        "HEPHAESTUS_PRIVATE_VOLUME_LEASE_SQL",
        "HEPHAESTUS_PRIVATE_RUN_CLEANUP_SQL",
    ] {
        let path = std::env::var(variable).expect("explicit private migration file");
        let sql = std::fs::read_to_string(path).expect("task-owned private SQL");
        sqlx::raw_sql(&sql)
            .execute(pool)
            .await
            .expect("private108 fragment");
    }
}

fn checkpoint(version: i64) -> Migrator {
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

pub async fn worker(pool: &PgPool) -> Arc<PostgresVolumeMetadataRepository> {
    Arc::new(PostgresVolumeMetadataRepository::new(
        worker_pool(pool).await,
    ))
}

pub async fn worker_pool(pool: &PgPool) -> PgPool {
    PgPoolOptions::new()
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
        .expect("actual worker pool")
}

pub async fn normal_run(
    pool: &PgPool,
    fixture: &Fixture,
    instance: Uuid,
    revision: Uuid,
    agent: Uuid,
) -> RunId {
    let existing: Option<Uuid> = sqlx::query_scalar("SELECT id FROM agent_attachments WHERE instance_id=$1 AND repository_id=$2 AND ref_selector='refs/heads/main'")
        .bind(instance).bind(fixture.consuming_repository).fetch_optional(pool).await.expect("existing exact attachment");
    let attachment = existing.unwrap_or_else(Uuid::new_v4);
    if existing.is_none() {
        sqlx::query("INSERT INTO agent_attachments(id,instance_id,project_id,repository_id,ref_selector,trigger_policy,created_by) VALUES($1,$2,$3,$4,'refs/heads/main','manual',$5)")
            .bind(attachment).bind(instance).bind(fixture.consuming_project).bind(fixture.consuming_repository).bind(fixture.maintainer.as_uuid()).execute(pool).await.expect("exact attachment");
    }
    let release: Uuid = sqlx::query_scalar("SELECT release_id FROM release_agents WHERE id=$1")
        .bind(agent)
        .fetch_one(pool)
        .await
        .expect("exact released export");
    let run = RunId::new();
    // Deliberately false command flag: authoritative release selection must win.
    sqlx::query("INSERT INTO runs(id,instance_id,instance_revision_id,release_id,release_agent_id,attachment_id,run_kind,command_id,state,requires_state,created_at,updated_at) VALUES($1,$2,$3,$4,$5,$6,'normal',$7,'leasing_volume',false,now(),now())")
        .bind(run.as_uuid()).bind(instance).bind(revision).bind(release).bind(agent).bind(attachment).bind(Uuid::new_v4()).execute(pool).await.expect("exact leasing run");
    run
}

pub async fn count(pool: &PgPool, run: RunId) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases WHERE run_id=$1 AND released_at IS NULL",
    )
    .bind(run.as_uuid())
    .fetch_one(pool)
    .await
    .expect("complete canonical set count")
}

pub fn times() -> (OffsetDateTime, OffsetDateTime) {
    let now = OffsetDateTime::now_utc();
    (now, now + time::Duration::minutes(1))
}
