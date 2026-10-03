use std::collections::BTreeMap;

use heph_run::{Run, RunKind, StartRun};
use run_orchestrator::RunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
use sqlx::PgPool;
use uuid::Uuid;
use volume_domain::RunVolumeIdentity;

use super::super::{PgAgentVmSpecFactory, RootFilesystem, RuntimePolicy};
use super::seed;

pub struct Fixture {
    pub run: Run,
    pub project: Uuid,
    pub volume: Option<Uuid>,
    pub factory: PgAgentVmSpecFactory,
}

impl Fixture {
    pub async fn new(pool: &PgPool, requires_state: bool) -> Self {
        let command = StartRun {
            command_id: CommandId::new(),
            run_id: RunId::new(),
            instance_id: AgentInstanceId::new(),
            instance_revision_id: AgentInstanceRevisionId::new(),
            release_id: ReleaseId::new(),
            release_agent_id: ReleaseAgentId::new(),
            attachment_id: Some(AgentAttachmentId::new()),
            kind: RunKind::Normal,
            requires_state,
        };
        let project = seed::seed_instance(pool, &command).await;
        let volume = if requires_state {
            Some(seed_volume(pool, command.instance_id, project).await)
        } else {
            None
        };
        let created = run_postgres::PgRunRepository::new(pool.clone())
            .create_run(&command)
            .await
            .expect("PUBLIC107 ordinary legacy run");
        assert!(created.created);
        Self {
            run: created.run,
            project,
            volume,
            factory: PgAgentVmSpecFactory {
                pool: pool.clone(),
                root_images: BTreeMap::from([(
                    "fixture-image".into(),
                    RootFilesystem::Directory {
                        // This test constructs specs only; it does not materialize an image.
                        host_path: "/unused-contract-test-image".into(),
                    },
                )]),
                runtime_policy: RuntimePolicy {
                    version: "test/v1".into(),
                    max_vcpus: 2,
                    max_memory_mib: 1_024,
                    allow_broker_only: false,
                    allow_egress: false,
                },
            },
        }
    }

    pub fn identity(&self) -> RunVolumeIdentity {
        RunVolumeIdentity::new(
            self.run.id,
            self.run.instance_id,
            self.run.instance_revision_id,
            self.run.release_id,
            self.run.release_agent_id,
            self.project,
        )
        .unwrap()
    }
}

async fn seed_volume(pool: &PgPool, instance: AgentInstanceId, project: Uuid) -> Uuid {
    let volume = Uuid::new_v4();
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id,instance_id,project_id,capacity_bytes,filesystem_uuid,state)
         VALUES($1,$2,$3,16777216,$4,'uninitialized')",
    )
    .bind(volume)
    .bind(instance.as_uuid())
    .bind(project)
    .bind(Uuid::new_v4())
    .execute(pool)
    .await
    .expect("legacy metadata without physical backing");
    sqlx::query("UPDATE agent_instances SET state_volume_id=$2 WHERE id=$1")
        .bind(instance.as_uuid())
        .bind(volume)
        .execute(pool)
        .await
        .expect("exact legacy pointer");
    volume
}
