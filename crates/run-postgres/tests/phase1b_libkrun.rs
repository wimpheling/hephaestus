//! Opt-in full Phase 1B persistence test requiring KVM and `PostgreSQL`.

use authz_postgres::PostgresMelangeAuthorizer;
use release_domain::AgentUpdateId;
use release_postgres::{ReleaseService, UpdateDecision};
use run_domain::{Run, RunKind, RunOutcome, RunState, StartRun};
use run_orchestrator::{RepositoryError, RunOrchestrator, RunRepository, VmSpecFactory};
use run_postgres::PgRunRepository;
use runtime_types::{
    AgentAttachmentId, AgentInstanceId, AgentInstanceRevisionId, CommandId, ReleaseAgentId,
    ReleaseId, RunId,
};
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
use volume_postgres::PostgresVolumeMetadataRepository;
use volume_trait::VolumeStore;

const ENABLE_FLAG: &str = "HEPHAESTUS_PHASE1B_INTEGRATION";

#[tokio::test(flavor = "multi_thread")]
// The sequential end-to-end scenario intentionally keeps all external
// resources alive through both runs so persistence and cleanup are observable.
#[allow(clippy::too_many_lines)]
async fn real_state_runs_and_update_outcomes_are_isolated_and_reconciled() {
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
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("runtime migrations");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(pool.clone())),
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
    let instance_id = AgentInstanceId::new();
    let scenario = seed_instance(&pool, instance_id).await;
    let factory = Arc::new(StateSpecFactory {
        rootfs,
        rollback_release: scenario.rejected.release,
        timeout_release: scenario.uncertain.release,
    });
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

    let first = command(scenario.current);
    let first_run = orchestrator
        .start_run(&first)
        .await
        .expect("first persistent run");
    assert_eq!(first_run.state, RunState::CleanedUp);
    assert_eq!(sqlite_previous(&pool, first.run_id).await, 0);
    let volume_id = first_run.volume_id.expect("first run volume");
    let backing_path: String =
        sqlx::query_scalar("SELECT host_path FROM agent_instance_state_volumes WHERE id = $1")
            .bind(volume_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("volume backing path");
    assert!(PathBuf::from(&backing_path).is_file());

    let second = command(scenario.current);
    let running_orchestrator = Arc::clone(&orchestrator);
    let second_for_task = second.clone();
    let running =
        tokio::spawn(async move { running_orchestrator.start_run(&second_for_task).await });
    wait_for_state(&repository, second.run_id, RunState::Running).await;
    let concurrent = command(scenario.current);
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
        "SELECT count(*) FROM agent_instance_volume_leases WHERE volume_id = $1 AND released_at IS NULL",
    )
    .bind(volume_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active lease count");
    assert_eq!(active_leases, 0);
    let successful_hook = update_command(scenario.successful);
    let successful_run = orchestrator
        .start_run(&successful_hook)
        .await
        .expect("successful real update hook");
    assert_eq!(successful_run.outcome, Some(RunOutcome::Succeeded));
    assert_eq!(sqlite_previous(&pool, successful_hook.run_id).await, 2);
    let successful_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        successful_update,
        scenario.current,
        scenario.successful,
        successful_hook.run_id,
    )
    .await;
    let releases = ReleaseService::new(pool.clone(), Arc::new(PostgresMelangeAuthorizer));
    assert_eq!(
        releases
            .reconcile_update_run(successful_hook.run_id)
            .await
            .expect("activate successful real hook"),
        UpdateDecision::Activated
    );
    let post_update = command(scenario.successful);
    let post_update_run = orchestrator
        .start_run(&post_update)
        .await
        .expect("normal run from activated candidate");
    assert_eq!(sqlite_previous(&pool, post_update.run_id).await, 3);
    assert!(
        log_contains(
            &pool,
            post_update.run_id,
            &format!("release_marker={}", scenario.successful.release)
        )
        .await,
        "post-update run must execute the candidate release contract"
    );

    let rejected_hook = update_command(scenario.rejected);
    let rejected_run = orchestrator
        .start_run(&rejected_hook)
        .await
        .expect("explicitly rejected real update hook");
    assert_eq!(rejected_run.outcome, Some(RunOutcome::Failed));
    assert_eq!(
        rejected_run.exit.as_ref().and_then(|exit| exit.code),
        Some(23)
    );
    let rejected_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        rejected_update,
        scenario.successful,
        scenario.rejected,
        rejected_hook.run_id,
    )
    .await;
    assert_eq!(
        releases
            .reconcile_update_run(rejected_hook.run_id)
            .await
            .expect("preserve current revision after agent rollback"),
        UpdateDecision::AgentRejected
    );
    let after_rejection = command(scenario.successful);
    let after_rejection_run = orchestrator
        .start_run(&after_rejection)
        .await
        .expect("normal run after explicit rollback");
    assert_eq!(
        sqlite_previous(&pool, after_rejection.run_id).await,
        4,
        "the rejected hook must not retain its transactional mutation"
    );

    let uncertain_hook = update_command(scenario.uncertain);
    let uncertain_run = orchestrator
        .start_run(&uncertain_hook)
        .await
        .expect("forced update termination is durably cleaned");
    assert_eq!(uncertain_run.outcome, Some(RunOutcome::Failed));
    assert!(
        uncertain_run
            .failure
            .as_deref()
            .is_some_and(|failure| failure.contains("wall-clock timeout"))
    );
    let uncertain_update = AgentUpdateId::new();
    stage_update_result(
        &pool,
        uncertain_update,
        scenario.successful,
        scenario.uncertain,
        uncertain_hook.run_id,
    )
    .await;
    assert_eq!(
        releases
            .reconcile_update_run(uncertain_hook.run_id)
            .await
            .expect("pause after forced update termination"),
        UpdateDecision::CompatibilityUnknown
    );
    let paused: (uuid::Uuid, String, bool) = sqlx::query_as(
        "SELECT active_revision_id, state, run_gate_open
         FROM agent_instances WHERE id = $1",
    )
    .bind(instance_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("paused uncertain instance");
    assert_eq!(
        paused,
        (
            scenario.successful.revision.as_uuid(),
            String::from("paused_unknown_state"),
            false,
        )
    );

    for run in [
        &first_run,
        &second_run,
        &successful_run,
        &post_update_run,
        &rejected_run,
        &after_rejection_run,
        &uncertain_run,
    ] {
        let vm_id = run.vm_id.as_deref().expect("run VM ID");
        assert!(!runtime_root.join(vm_id).exists());
        assert!(!cgroup_root.join(vm_id).exists());
    }
    assert!(PathBuf::from(&backing_path).is_file());
    let active_leases_after_updates: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM agent_instance_volume_leases
         WHERE volume_id = $1 AND released_at IS NULL",
    )
    .bind(volume_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("active lease count after updates");
    assert_eq!(active_leases_after_updates, 0);
    let retained_releases: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM releases
         WHERE id = ANY($1)",
    )
    .bind(vec![
        scenario.current.release.as_uuid(),
        scenario.successful.release.as_uuid(),
        scenario.rejected.release.as_uuid(),
        scenario.uncertain.release.as_uuid(),
    ])
    .fetch_one(&pool)
    .await
    .expect("retained update releases");
    assert_eq!(retained_releases, 4);

    cleanup_runtime_records(&pool, instance_id).await;
}

#[derive(Clone, Copy)]
struct RunTarget {
    instance: AgentInstanceId,
    revision: AgentInstanceRevisionId,
    release: ReleaseId,
    release_agent: ReleaseAgentId,
    attachment: AgentAttachmentId,
}

#[derive(Clone, Copy)]
struct UpdateScenario {
    current: RunTarget,
    successful: RunTarget,
    rejected: RunTarget,
    uncertain: RunTarget,
}

#[allow(clippy::too_many_lines)]
async fn seed_instance(pool: &PgPool, instance_id: AgentInstanceId) -> UpdateScenario {
    let organization_id = uuid::Uuid::new_v4();
    let project_id = uuid::Uuid::new_v4();
    let repository_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let build_id = uuid::Uuid::new_v4();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    let attachment_id = AgentAttachmentId::new();
    let state_volume_id = uuid::Uuid::new_v4();
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
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(repository_id)
    .bind(project_id)
    .bind(format!("repository-{repository_id}"))
    .execute(pool)
    .await
    .expect("libkrun repository");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'state-agent')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("libkrun family");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5, '{}',
                 $6, $7, 'published', now())",
    )
    .bind(release_id.as_uuid())
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state)
         VALUES ($1, $2, $3, 'state-agent', 'State Agent', $4, $5, true)",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "phase1b"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(family_id)
    .bind(format!("agent-{instance_id}"))
    .execute(pool)
    .await
    .expect("libkrun instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
         (id, instance_id, state, capacity_bytes)
         VALUES ($1, $2, 'uninitialized', $3)",
    )
    .bind(state_volume_id)
    .bind(instance_id.as_uuid())
    .bind(128_i64 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("libkrun instance state volume");
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(state_volume_id)
        .execute(pool)
        .await
        .expect("attach libkrun instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'phase1b/v1', true)",
    )
    .bind(revision_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind(serde_json::json!({"network": "egress"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("libkrun revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id.as_uuid())
        .bind(revision_id.as_uuid())
        .execute(pool)
        .await
        .expect("libkrun active revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'manual')",
    )
    .bind(attachment_id.as_uuid())
    .bind(instance_id.as_uuid())
    .bind(project_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("libkrun attachment");
    let current = RunTarget {
        instance: instance_id,
        revision: revision_id,
        release: release_id,
        release_agent: release_agent_id,
        attachment: attachment_id,
    };
    UpdateScenario {
        current,
        successful: seed_candidate(pool, current, repository_id, family_id, "v2", 'b').await,
        rejected: seed_candidate(pool, current, repository_id, family_id, "v3", 'c').await,
        uncertain: seed_candidate(pool, current, repository_id, family_id, "v4", 'd').await,
    }
}

async fn seed_candidate(
    pool: &PgPool,
    current: RunTarget,
    repository_id: uuid::Uuid,
    family_id: uuid::Uuid,
    version: &str,
    commit_character: char,
) -> RunTarget {
    let build_id = uuid::Uuid::new_v4();
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let revision_id = AgentInstanceRevisionId::new();
    let commit = commit_character.to_string().repeat(40);
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind(&commit)
    .bind([11_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate build");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', $5, $6, '{}',
                 $7, $8, 'published', now())",
    )
    .bind(release_id.as_uuid())
    .bind(repository_id)
    .bind(version)
    .bind(commit)
    .bind(build_id)
    .bind([11_u8; 32].as_slice())
    .bind([12_u8; 32].as_slice())
    .bind([13_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, requires_state, update_hook)
         VALUES ($1, $2, $3, 'state-agent', 'State Agent', $4, $5, true, $6)",
    )
    .bind(release_agent_id.as_uuid())
    .bind(release_id.as_uuid())
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "phase1b"
    }))
    .bind([14_u8; 32].as_slice())
    .bind(serde_json::json!({
        "command": "bin/update",
        "arguments": [],
        "working_directory": "."
    }))
    .execute(pool)
    .await
    .expect("candidate release agent");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $7, $8, 'phase1b/v2', true)",
    )
    .bind(revision_id.as_uuid())
    .bind(current.instance.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind([15_u8; 32].as_slice())
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind(serde_json::json!({"network": "egress"}))
    .bind(serde_json::json!({"vcpus": 1, "memory_mib": 512, "network": "egress"}))
    .bind([16_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("candidate instance revision");
    RunTarget {
        instance: current.instance,
        revision: revision_id,
        release: release_id,
        release_agent: release_agent_id,
        attachment: current.attachment,
    }
}

struct StateSpecFactory {
    rootfs: PathBuf,
    rollback_release: ReleaseId,
    timeout_release: ReleaseId,
}

#[async_trait::async_trait]
impl VmSpecFactory for StateSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        let (argument, timeout) = if run.release_id == self.rollback_release {
            ("--state-rollback", false)
        } else if run.release_id == self.timeout_release {
            ("--ignore-cancellation", true)
        } else {
            ("--state-only", false)
        };
        let mut labels = BTreeMap::new();
        if timeout {
            labels.insert(
                String::from("hephaestus.wall-clock-timeout-seconds"),
                String::from("1"),
            );
        }
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
                args: vec![String::from(argument)],
                env: BTreeMap::from([
                    (
                        String::from("HEPH_RELEASE_MARKER"),
                        run.release_id.to_string(),
                    ),
                    (
                        String::from("HEPH_STATE_HOLD_MS"),
                        if run.kind == RunKind::Normal {
                            String::from("1000")
                        } else {
                            String::from("0")
                        },
                    ),
                ]),
                working_dir: None,
            },
            labels,
        })
    }
}

fn command(target: RunTarget) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: target.instance,
        instance_revision_id: target.revision,
        release_id: target.release,
        release_agent_id: target.release_agent,
        attachment_id: Some(target.attachment),
        kind: RunKind::Normal,
        requires_state: true,
    }
}

fn update_command(target: RunTarget) -> StartRun {
    StartRun {
        command_id: CommandId::new(),
        run_id: RunId::new(),
        instance_id: target.instance,
        instance_revision_id: target.revision,
        release_id: target.release,
        release_agent_id: target.release_agent,
        attachment_id: None,
        kind: RunKind::Update,
        requires_state: true,
    }
}

async fn stage_update_result(
    pool: &PgPool,
    update_id: AgentUpdateId,
    expected: RunTarget,
    candidate: RunTarget,
    hook_run_id: RunId,
) {
    sqlx::query(
        "UPDATE agent_instances
         SET state = 'updating', run_gate_open = false, updated_at = now()
         WHERE id = $1 AND active_revision_id = $2",
    )
    .bind(expected.instance.as_uuid())
    .bind(expected.revision.as_uuid())
    .execute(pool)
    .await
    .expect("close run gate for completed real hook");
    sqlx::query(
        "INSERT INTO agent_updates
         (id, instance_id, expected_current_revision_id,
          candidate_revision_id, state, hook_run_id)
         VALUES ($1, $2, $3, $4, 'hook_running', $5)",
    )
    .bind(update_id.as_uuid())
    .bind(expected.instance.as_uuid())
    .bind(expected.revision.as_uuid())
    .bind(candidate.revision.as_uuid())
    .bind(hook_run_id.as_uuid())
    .execute(pool)
    .await
    .expect("associate real hook with durable update");
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

async fn log_contains(pool: &PgPool, run_id: RunId, expected: &str) -> bool {
    let payloads: Vec<Value> = sqlx::query_scalar(
        "SELECT payload FROM run_events
         WHERE run_id = $1 AND event_type = 'vm.log'
         ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    .expect("run log payloads");
    payloads
        .iter()
        .filter_map(|payload| payload.get("bytes").and_then(Value::as_array))
        .flatten()
        .filter_map(Value::as_u64)
        .filter_map(|byte| u8::try_from(byte).ok())
        .map(char::from)
        .collect::<String>()
        .contains(expected)
}

async fn cleanup_runtime_records(pool: &PgPool, instance_id: AgentInstanceId) {
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_id IN (SELECT id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean outbox");
    sqlx::query(
        "DELETE FROM run_events WHERE run_id IN (SELECT id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean run events");
    sqlx::query(
        "DELETE FROM command_inbox WHERE command_id IN
         (SELECT command_id FROM runs WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean command inbox");
    sqlx::query(
        "DELETE FROM outbox
         WHERE aggregate_id IN (
             SELECT id FROM agent_updates WHERE instance_id = $1
         )",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean update outbox");
    sqlx::query("DELETE FROM agent_updates WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean updates");
    sqlx::query("UPDATE runs SET lease_id = NULL WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("detach run lease provenance for fixture cleanup");
    sqlx::query(
        "DELETE FROM agent_instance_volume_leases WHERE volume_id IN
         (SELECT id FROM agent_instance_state_volumes WHERE instance_id = $1)",
    )
    .bind(instance_id.as_uuid())
    .execute(pool)
    .await
    .expect("clean volume leases");
    sqlx::query("DELETE FROM runs WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean runs");
    sqlx::query("UPDATE agent_instances SET state_volume_id = NULL WHERE id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("detach instance volume");
    sqlx::query("DELETE FROM agent_instance_state_volumes WHERE instance_id = $1")
        .bind(instance_id.as_uuid())
        .execute(pool)
        .await
        .expect("clean volume");
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} must be set when {ENABLE_FLAG}=1"),
        PathBuf::from,
    )
}
