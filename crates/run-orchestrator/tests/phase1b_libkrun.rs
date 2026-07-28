//! Opt-in full Phase 1B persistence test requiring KVM and `PostgreSQL`.

use run_domain::{Run, RunOutcome, RunState, StartRun};
use run_orchestrator::{
    PgRunRepository, RepositoryError, RunOrchestrator, RunRepository, VmSpecFactory,
};
use runtime_types::{AgentId, CommandId, RunId};
use serde_json::Value;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use vm_libkrun::{LibkrunConfig, LibkrunProvider};
use vm_trait::{
    GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmProvider, VmResources, VmSpec,
};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_trait::VolumeStore;

const ENABLE_FLAG: &str = "HEPHAESTUS_PHASE1B_INTEGRATION";

#[tokio::test(flavor = "multi_thread")]
// The sequential end-to-end scenario intentionally keeps all external
// resources alive through both runs so persistence and cleanup are observable.
#[allow(clippy::too_many_lines)]
async fn two_runs_share_state_and_reject_concurrent_writer() {
    if env::var(ENABLE_FLAG).as_deref() != Ok("1") {
        return;
    }
    let database_url = env::var("HEPHAESTUS_POSTGRES_TEST_URL").expect("Phase 1B PostgreSQL URL");
    let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
    let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
    let rootfs = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
    let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
    let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
    let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
    let worker = required_path("HEPHAESTUS_LIBKRUN_WORKER");
    let volume_root = disk_root.join("phase1b-volumes");
    fs::create_dir_all(&volume_root).expect("persistent test volume root");
    let volume_root = volume_root.canonicalize().expect("canonical volume root");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect Phase 1B PostgreSQL");
    let repository = Arc::new(PgRunRepository::new(pool.clone()));
    repository.initialize().await.expect("runtime migrations");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            pool.clone(),
            LocalVolumeConfig {
                volume_root: volume_root.clone(),
                transient_runtime_roots: vec![runtime_root.clone()],
                host_id: String::from("phase1b-integration-host"),
                lease_duration: Duration::from_secs(10),
                mkfs_ext4: PathBuf::from("/usr/bin/mkfs.ext4"),
            },
        )
        .expect("volume configuration"),
    );
    volumes.initialize().await.expect("volume initialization");
    let mut provider_config = LibkrunConfig::new(
        &runtime_root,
        vec![image_root],
        vec![disk_root],
        vec![mount_root],
        worker,
        &cgroup_root,
    );
    provider_config.startup_timeout = Duration::from_secs(15);
    provider_config.readiness_timeout = Duration::from_secs(45);
    let provider = Arc::new(LibkrunProvider::new(provider_config).expect("libkrun provider"));
    let factory = Arc::new(StateSpecFactory { rootfs });
    let repository_trait: Arc<dyn RunRepository> = repository.clone();
    let volume_trait: Arc<dyn VolumeStore> = volumes.clone();
    let provider_trait: Arc<dyn VmProvider> = provider;
    let orchestrator = Arc::new(RunOrchestrator::new(
        repository_trait,
        volume_trait,
        provider_trait,
        factory,
        128 * 1024 * 1024,
    ));

    let agent_id = AgentId::new();
    seed_agent(&pool, agent_id).await;
    let first = command(agent_id);
    let first_run = orchestrator
        .start_run(&first)
        .await
        .expect("first persistent run");
    assert_eq!(first_run.state, RunState::CleanedUp);
    assert_eq!(sqlite_previous(&pool, first.run_id).await, 0);
    let volume_id = first_run.volume_id.expect("first run volume");
    let backing_path: String =
        sqlx::query_scalar("SELECT host_path FROM agent_state_volumes WHERE id = $1")
            .bind(volume_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("volume backing path");
    assert!(PathBuf::from(&backing_path).is_file());

    let second = command(agent_id);
    let running_orchestrator = Arc::clone(&orchestrator);
    let second_for_task = second.clone();
    let running =
        tokio::spawn(async move { running_orchestrator.start_run(&second_for_task).await });
    wait_for_state(&repository, second.run_id, RunState::Running).await;
    let concurrent = command(agent_id);
    let rejected = orchestrator
        .start_run(&concurrent)
        .await
        .expect("durably reject concurrent run");
    assert_eq!(rejected.state, RunState::CleanedUp);
    assert_eq!(rejected.outcome, Some(RunOutcome::Failed));
    assert!(
        rejected
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("leased by run"))
    );
    let second_run = running
        .await
        .expect("join second run")
        .expect("second persistent run");
    assert_eq!(second_run.state, RunState::CleanedUp);
    assert_eq!(sqlite_previous(&pool, second.run_id).await, 1);
    let active_leases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM volume_leases WHERE volume_id = $1 AND released_at IS NULL",
    )
    .bind(volume_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active lease count");
    assert_eq!(active_leases, 0);
    for run in [&first_run, &second_run] {
        let vm_id = run.vm_id.as_deref().expect("run VM ID");
        assert!(!runtime_root.join(vm_id).exists());
        assert!(!cgroup_root.join(vm_id).exists());
    }
    assert!(PathBuf::from(&backing_path).is_file());

    cleanup_database(&pool, agent_id).await;
}

async fn seed_agent(pool: &PgPool, agent_id: AgentId) {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id)
        .bind(format!("libkrun-{organization_id}"))
        .execute(pool)
        .await
        .expect("libkrun organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project_id)
        .bind(organization_id)
        .bind(format!("libkrun-{project_id}"))
        .execute(pool)
        .await
        .expect("libkrun project");
    sqlx::query("INSERT INTO agents (id, project_id, name) VALUES ($1, $2, $3)")
        .bind(agent_id.as_uuid())
        .bind(project_id)
        .bind(format!("agent-{agent_id}"))
        .execute(pool)
        .await
        .expect("libkrun agent");
}

struct StateSpecFactory {
    rootfs: PathBuf,
}

#[async_trait::async_trait]
impl VmSpecFactory for StateSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: self.rootfs.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 512,
            },
            // Exercise `passt` lifecycle cleanup as part of the full
            // acceptance scenario, without exposing any ingress port.
            network: NetworkMode::UserMode {
                ingress: Vec::new(),
            },
            command: GuestCommand {
                program: String::from("/usr/libexec/hephaestus/integration-check"),
                args: vec![String::from("--state-only")],
                env: BTreeMap::from([(String::from("HEPH_STATE_HOLD_MS"), String::from("1000"))]),
                working_dir: None,
            },
            labels: BTreeMap::new(),
        })
    }
}

fn command(agent_id: AgentId) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        agent_id,
    }
}

async fn wait_for_state(repository: &PgRunRepository, run_id: RunId, expected: RunState) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        match repository.get(run_id).await {
            Ok(run) if run.state == expected => return,
            Ok(_) | Err(RepositoryError::NotFound(_)) => {}
            Err(error) => panic!("poll run: {error}"),
        }
        assert!(Instant::now() < deadline, "run never reached {expected:?}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

async fn sqlite_previous(pool: &PgPool, run_id: RunId) -> u64 {
    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM run_events
         WHERE run_id = $1 AND event_type = 'vm.log'
         ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("run log payloads");
    let text = payloads
        .iter()
        .filter_map(|payload| payload.get("bytes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|byte| u8::try_from(byte).ok())
        .map(char::from)
        .collect::<String>();
    text.lines()
        .find_map(|line| line.strip_prefix("sqlite_previous="))
        .expect("SQLite previous-row marker")
        .parse()
        .expect("valid SQLite previous-row marker")
}

async fn cleanup_database(pool: &PgPool, agent_id: AgentId) {
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_id IN (SELECT id FROM runs WHERE agent_id = $1)",
    )
    .bind(agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean outbox");
    sqlx::query("DELETE FROM run_events WHERE run_id IN (SELECT id FROM runs WHERE agent_id = $1)")
        .bind(agent_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean run events");
    sqlx::query(
        "DELETE FROM command_inbox WHERE command_id IN
         (SELECT command_id FROM runs WHERE agent_id = $1)",
    )
    .bind(agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean command inbox");
    sqlx::query("DELETE FROM runs WHERE agent_id = $1")
        .bind(agent_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean runs");
    sqlx::query(
        "DELETE FROM volume_leases WHERE volume_id IN
         (SELECT id FROM agent_state_volumes WHERE agent_id = $1)",
    )
    .bind(agent_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean volume leases");
    sqlx::query("DELETE FROM agent_state_volumes WHERE agent_id = $1")
        .bind(agent_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean volume");
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(agent_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean agent");
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}
