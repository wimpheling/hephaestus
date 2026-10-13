use run_domain::{FreshRunExecutionProfile, RunState, StartRun};
use run_orchestrator::RunRepository;
use run_postgres::PgRunRepository;
use runtime_types::VolumeId;
use sqlx::PgPool;
use time::OffsetDateTime;
use volume_postgres::{PostgresVolumeMetadataRepository, ScalarMetadataSchema};
use volume_trait::{VolumeLease, VolumeMetadataRepository};

use super::public_fixture::{Role, role_pool};

pub use super::public_fixture::command;

pub async fn schema(pool: &PgPool) {
    assert_eq!(
        std::env::var("HEPHAESTUS_PRIVATE_RECIPE_MIGRATIONS").as_deref(),
        Ok("1")
    );
    super::public_fixture::checkpoint(107)
        .run(pool)
        .await
        .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0108_run_volume_leases.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0109_run_cleanup.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0110_legacy_scalar_compatibility.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
    sqlx::raw_sql(include_str!(
        "../../../../../../migrations/0111_run_profile_lifecycle.sql"
    ))
    .execute(pool)
    .await
    .unwrap();
}

pub struct Fixture {
    pub command: StartRun,
    pub worker: PgPool,
    pub runs: PgRunRepository,
    pub volumes: PostgresVolumeMetadataRepository,
    pub volume: Option<VolumeId>,
}

impl Fixture {
    pub async fn new(pool: &PgPool, requires_state: bool) -> Self {
        let mut command = super::public_fixture::command();
        command.requires_state = requires_state;
        super::support::seed_instance(pool, &command).await;
        super::support::seed_run_request(pool, &command).await;
        let volume = if requires_state {
            let volume = VolumeId::new();
            sqlx::query("INSERT INTO agent_instance_state_volumes(id,instance_id,project_id,state,host_id,host_path,capacity_bytes,filesystem_uuid,provisioning_state) SELECT $1,id,project_id,'ready','lifecycle-host',$2,16777216,gen_random_uuid(),'ready' FROM agent_instances WHERE id=$3")
                .bind(volume.as_uuid()).bind(format!("/controlled-lifecycle/{volume}"))
                .bind(command.instance_id.as_uuid()).execute(pool).await.unwrap();
            sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
                .bind(command.instance_id.as_uuid())
                .bind(volume.as_uuid())
                .execute(pool)
                .await
                .unwrap();
            Some(volume)
        } else {
            None
        };
        let worker = role_pool(pool, Role::Worker).await;
        let runs = PgRunRepository::new(worker.clone())
            .with_execution_profile(FreshRunExecutionProfile::LegacyScalar);
        let volumes = PostgresVolumeMetadataRepository::new(worker.clone())
            .with_scalar_schema(ScalarMetadataSchema::Profiled);
        Self {
            command,
            worker,
            runs,
            volumes,
            volume,
        }
    }

    pub async fn acquire(&self) -> Option<VolumeLease> {
        self.runs.create_run(&self.command).await.unwrap();
        self.runs
            .transition(self.command.run_id, RunState::LeasingVolume, None, None)
            .await
            .unwrap();
        let volume = self.volume?;
        let now = OffsetDateTime::now_utc();
        Some(
            self.volumes
                .acquire(
                    volume,
                    self.command.run_id,
                    "lifecycle-host",
                    now,
                    now + time::Duration::minutes(5),
                )
                .await
                .unwrap(),
        )
    }

    pub async fn bind(&self, lease: Option<&VolumeLease>) {
        self.runs
            .bind_resources(
                self.command.run_id,
                lease.map(|l| l.volume_id),
                lease.map(|l| l.id),
                lease.map(|l| l.fencing_token),
                &self.command.run_id.to_string(),
            )
            .await
            .unwrap();
    }

    pub async fn to_running(&self, lease: Option<&VolumeLease>) {
        self.bind(lease).await;
        for state in [
            RunState::Provisioning,
            RunState::Starting,
            RunState::Running,
        ] {
            self.runs
                .transition(self.command.run_id, state, None, None)
                .await
                .unwrap();
        }
    }
}

pub async fn evidence(pool: &PgPool, run: runtime_types::RunId) -> serde_json::Value {
    sqlx::query_scalar("SELECT jsonb_build_object('run',to_jsonb(execution),'leases',(SELECT jsonb_agg(to_jsonb(lease) ORDER BY lease.id) FROM agent_instance_volume_leases lease WHERE lease.run_id=execution.id),'volumes',(SELECT jsonb_agg(to_jsonb(volume) ORDER BY volume.id) FROM agent_instance_state_volumes volume WHERE volume.id=execution.volume_id),'events',(SELECT count(*) FROM run_events WHERE run_id=execution.id)) FROM runs execution WHERE execution.id=$1")
        .bind(run.as_uuid()).fetch_one(pool).await.unwrap()
}
