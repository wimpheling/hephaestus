//! Opt-in `PostgreSQL` and `JetStream` receive-processing coverage.

use forge_domain::{CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate, Repository};
use forge_postgres::PgForgeRepository;
use forge_service::{
    CreateRepository, ForgeNatsOutboxPublisher, GitStorage, INSTANCE_RUN_REQUESTED_SUBJECT,
    ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{
    NatsCommandHandler, RunOrchestrator, RunRepository, VmSpecFactory, ensure_jetstream_topology,
};
use run_postgres::PgRunRepository;
use runtime_types::{CommandId, RunId};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio::process::Command;
use uuid::Uuid;
use vm_fake::FakeProvider;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmResources, VmSpec};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};
use volume_postgres::PostgresVolumeMetadataRepository;

#[tokio::test]
#[serial]
async fn persists_exact_config_and_deduplicates_receive() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let (commit, update) = commit_and_update(&temporary, &repository, valid_config()).await;
    let receive_id = ReceiveId::new();
    let first = service
        .accept_receive(
            &repository,
            receive_id,
            "integration-user",
            std::slice::from_ref(&update),
        )
        .await
        .expect("accepted receive");
    let duplicate = service
        .accept_receive(&repository, receive_id, "integration-user", &[update])
        .await
        .expect("duplicate receive");

    assert_eq!(first.run_requests.len(), 1);
    assert_eq!(duplicate.run_requests, first.run_requests);
    let stored_commit: String = sqlx::query_scalar(
        "SELECT commit_sha FROM agent_config_revisions WHERE repository_id = $1",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored config commit");
    let starts: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE aggregate_type = 'forge' AND subject = $1
           AND aggregate_id = $2",
    )
    .bind(INSTANCE_RUN_REQUESTED_SUBJECT)
    .bind(first.run_requests[0].id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("start command count");
    assert_eq!(stored_commit, commit.as_str());
    assert_eq!(starts, 1);
    let reusable_requests: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests
         WHERE receive_id = $1 AND request_kind = 'instance_normal'",
    )
    .bind(receive_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("exact reusable request count");
    assert_eq!(reusable_requests, 1);
    let reusable_events: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM outbox
         WHERE subject = 'hephaestus.instance.run.requested.v1'
           AND payload->>'receive_id' = $1",
    )
    .bind(receive_id.to_string())
    .fetch_one(&pool)
    .await
    .expect("reusable run event count");
    assert_eq!(reusable_events, 1);

    let work = temporary.path().join("work");
    let invalid = valid_config().replace("version = 2", "version = 99");
    tokio::fs::write(work.join("agent.toml"), invalid)
        .await
        .expect("invalid agent configuration");
    git(&work, &["add", "agent.toml"]).await;
    git(&work, &["commit", "-m", "unsupported config"]).await;
    let invalid_commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("invalid commit");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let invalid_result = service
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "integration-user",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
                old_commit: Some(commit),
                new_commit: Some(invalid_commit),
            }],
        )
        .await
        .expect("invalid configuration receive");
    assert_eq!(invalid_result.invalid_configurations, 1);
    assert_eq!(
        invalid_result.run_requests.len(),
        1,
        "target agent.toml validity must not control attached instances"
    );
    let diagnostic_code: String = sqlx::query_scalar(
        "SELECT diagnostics->0->>'code'
         FROM agent_config_revisions
         WHERE repository_id = $1 AND status = 'invalid'",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("stored diagnostic");
    assert_eq!(diagnostic_code, "unsupported_version");

    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
async fn forge_outbox_retry_is_deduplicated_by_jetstream() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    // Opt-in test binaries may share one explicitly configured database.
    sqlx::query(
        "UPDATE outbox SET published_at = now()
         WHERE aggregate_type = 'forge' AND published_at IS NULL",
    )
    .execute(&pool)
    .await
    .expect("isolate forge outbox fixture");
    seed_reusable_attachment(&pool, &repository).await;
    let (_, update) = commit_and_update(&temporary, &repository, valid_config()).await;
    service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let command_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM outbox
         WHERE published_at IS NULL
           AND subject IN (
               'hephaestus.build.requested.v1',
               'hephaestus.instance.run.requested.v1',
               'hephaestus.run.start'
           )
         ORDER BY occurred_at, id",
    )
    .fetch_all(&pool)
    .await
    .expect("exact receive command identities");
    assert!(!command_ids.is_empty());

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let stream_name = format!("HEPH_PHASE2_TEST_{}", repository.id.as_uuid().simple());
    let mut stream = context
        .create_stream(async_nats::jetstream::stream::Config {
            name: stream_name.clone(),
            subjects: vec![String::from("hephaestus.>")],
            duplicate_window: Duration::from_secs(60),
            ..Default::default()
        })
        .await
        .expect("isolated forge stream");
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("first publication"),
        command_ids.len()
    );
    assert_eq!(
        stream.info().await.expect("stream state").state.messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );
    sqlx::query("UPDATE outbox SET published_at = NULL WHERE id = ANY($1)")
        .bind(&command_ids)
        .execute(&pool)
        .await
        .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("retry publication"),
        command_ids.len()
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        u64::try_from(command_ids.len()).expect("bounded command count")
    );

    context
        .delete_stream(&stream_name)
        .await
        .expect("delete isolated stream");
    cleanup(&pool, repository).await;
}

#[tokio::test]
#[serial]
// This deliberately crosses every durable boundary in one scenario.
#[allow(clippy::too_many_lines)]
async fn pushed_config_publishes_command_and_starts_vm() {
    let (Ok(nats_url), Some((pool, service, repository, temporary))) =
        (std::env::var("HEPHAESTUS_NATS_TEST_URL"), fixture().await)
    else {
        return;
    };
    seed_reusable_attachment(&pool, &repository).await;
    let (_, update) = commit_and_update(&temporary, &repository, valid_config()).await;
    let receive = service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");
    let request = receive.run_requests[0].clone();

    let client = async_nats::connect(nats_url)
        .await
        .expect("NATS integration connection");
    let context = async_nats::jetstream::new(client);
    let consumer = ensure_jetstream_topology(&context)
        .await
        .expect("run topology");
    ensure_forge_jetstream_topology(&context)
        .await
        .expect("forge topology");

    let run_repository = Arc::new(PgRunRepository::new(pool.clone()));
    let volume_root = temporary.path().join("volumes");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            Arc::new(PostgresVolumeMetadataRepository::new(pool.clone())),
            LocalVolumeConfig {
                volume_root,
                transient_runtime_roots: Vec::new(),
                host_id: String::from("phase2-integration"),
                lease_duration: Duration::from_secs(30),
                mkfs_ext4: std::path::PathBuf::from("/usr/bin/mkfs.ext4"),
            },
        )
        .expect("volume configuration"),
    );
    volumes.initialize().await.expect("volume store");
    let guest_root = temporary.path().join("guest-root");
    tokio::fs::create_dir(&guest_root)
        .await
        .expect("guest root");
    let orchestrator = Arc::new(RunOrchestrator::new(
        run_repository.clone(),
        volumes,
        Arc::new(FakeProvider::new()),
        Arc::new(TestSpecFactory { root: guest_root }),
        16 * 1024 * 1024,
    ));
    let handler = NatsCommandHandler::new(Arc::clone(&orchestrator));
    let publisher = ForgeNatsOutboxPublisher::new(context.clone());
    assert!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("publish receive commands")
            > 0
    );

    let mut messages = consumer.messages().await.expect("command messages");
    let message = messages
        .next()
        .await
        .expect("start delivery")
        .expect("valid start delivery");
    let handler_task = tokio::spawn(async move { handler.handle(&message).await });
    wait_for_run_state(&pool, request.command.run_id, "running").await;
    orchestrator
        .cancel_run(&CancelRun {
            command_id: CommandId::new(),
            run_id: request.command.run_id,
            reason: String::from("complete integration test"),
        })
        .await
        .expect("stop started VM");
    handler_task
        .await
        .expect("join command handler")
        .expect("handle start command");
    assert_eq!(
        run_repository
            .get(request.command.run_id)
            .await
            .expect("completed run")
            .state,
        RunState::CleanedUp
    );
    let reached_running: bool = sqlx::query_scalar(
        "SELECT EXISTS(
           SELECT 1 FROM run_events
           WHERE run_id = $1 AND event_type = 'run.running'
         )",
    )
    .bind(request.command.run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("running transition");
    assert!(reached_running, "published command did not start the VM");

    cleanup_run(&pool, &request.command).await;
    cleanup(&pool, repository).await;
    context
        .delete_stream("HEPH_RUN_COMMANDS")
        .await
        .expect("delete command stream");
    context
        .delete_stream("HEPH_RUN_EVENTS")
        .await
        .expect("delete run event stream");
    context
        .delete_stream("HEPHAESTUS_GIT_EVENTS")
        .await
        .expect("delete Git event stream");
}

struct TestSpecFactory {
    root: std::path::PathBuf,
}

#[async_trait::async_trait]
impl VmSpecFactory for TestSpecFactory {
    async fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
        Ok(VmSpec {
            id: VmId(run.id.to_string()),
            root: RootFilesystem::Directory {
                host_path: self.root.clone(),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 128,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/bin/true"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            labels: BTreeMap::new(),
        })
    }
}

async fn wait_for_run_state(pool: &PgPool, run_id: RunId, expected: &str) {
    for _ in 0..200 {
        let state = sqlx::query_scalar::<_, String>("SELECT state FROM runs WHERE id = $1")
            .bind(run_id.as_uuid())
            .fetch_optional(pool)
            .await
            .expect("load run state");
        if state.as_deref() == Some(expected) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("run {run_id} did not reach {expected}");
}

async fn cleanup_run(pool: &PgPool, command: &StartRun) {
    let volume_id: Option<uuid::Uuid> =
        sqlx::query_scalar("SELECT volume_id FROM runs WHERE id = $1")
            .bind(command.run_id.as_uuid())
            .fetch_one(pool)
            .await
            .expect("run volume");
    sqlx::query("DELETE FROM outbox WHERE aggregate_type = 'run' AND aggregate_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run outbox");
    sqlx::query("DELETE FROM run_events WHERE run_id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run events");
    sqlx::query("DELETE FROM command_inbox WHERE payload->>'run_id' = $1")
        .bind(command.run_id.to_string())
        .execute(pool)
        .await
        .expect("delete command inbox");
    if let Some(volume_id) = volume_id {
        sqlx::query("DELETE FROM agent_instance_volume_leases WHERE volume_id = $1")
            .bind(volume_id)
            .execute(pool)
            .await
            .expect("delete volume leases");
    }
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run");
}

async fn fixture() -> Option<(PgPool, PgForgeRepository, Repository, tempfile::TempDir)> {
    let url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok()?;
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("PostgreSQL integration connection");
    let temporary = tempfile::tempdir().expect("temporary directory");
    let storage = Arc::new(
        GitStorage::initialize(temporary.path().join("repositories"))
            .await
            .expect("Git storage"),
    );
    let service = PgForgeRepository::new(pool.clone(), storage);
    service.initialize().await.expect("forge migrations");
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind("forge-service-integration")
        .execute(&pool)
        .await
        .expect("organization");
    let project = service
        .create_project_trusted(organization_id, "forge-service-integration")
        .await
        .expect("project");
    let repository = service
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");
    Some((pool, service, repository, temporary))
}

async fn seed_reusable_attachment(pool: &PgPool, repository: &Repository) {
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    seed_reusable_release(
        pool,
        repository,
        build_id,
        family_id,
        release_id,
        release_agent_id,
    )
    .await;
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let attachment_id = Uuid::new_v4();
    seed_attached_instance(
        pool,
        repository,
        family_id,
        release_agent_id,
        instance_id,
        revision_id,
        attachment_id,
    )
    .await;
}

async fn seed_reusable_release(
    pool: &PgPool,
    repository: &Repository,
    build_id: Uuid,
    family_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded')",
    )
    .bind(build_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind([1_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'attached')",
    )
    .bind(family_id)
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 '{}', $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(repository.id.as_uuid())
    .bind("d".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'attached', 'Attached', '{}', $4,
                 '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind([4_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed release agent");
}

// Explicit fixture IDs keep every seeded foreign-key edge visible in the test.
#[allow(clippy::too_many_arguments)]
async fn seed_attached_instance(
    pool: &PgPool,
    repository: &Repository,
    family_id: Uuid,
    release_agent_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
) {
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(family_id)
    .bind(format!("attached-{}", instance_id.simple()))
    .execute(pool)
    .await
    .expect("seed instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction,
          effective_runtime_policy, effective_policy_hash,
          platform_policy_version, runnable)
         VALUES ($1, $2, $3, '{}', $4, '{}', '{}', '{}', $5,
                 'platform/v1', true)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push')",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(repository.project_id.as_uuid())
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("seed attachment");
}

async fn commit_and_update(
    temporary: &tempfile::TempDir,
    repository: &Repository,
    config: &str,
) -> (CommitSha, RefUpdate) {
    let work = temporary.path().join("work");
    tokio::fs::create_dir(&work).await.expect("work directory");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Hephaestus Test"]).await;
    git(
        &work,
        &["config", "user.email", "hephaestus@example.invalid"],
    )
    .await;
    tokio::fs::write(work.join("agent.toml"), config)
        .await
        .expect("agent configuration");
    git(&work, &["add", "agent.toml"]).await;
    git(&work, &["commit", "-m", "agent config"]).await;
    let commit =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("commit ID");
    let bare = temporary
        .path()
        .join("repositories")
        .join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    (
        commit.clone(),
        RefUpdate {
            git_ref: GitRef::parse("refs/heads/main").expect("updated ref"),
            old_commit: None,
            new_commit: Some(commit),
        },
    )
}

async fn cleanup(pool: &PgPool, repository: Repository) {
    let retains_instance_history: bool = sqlx::query_scalar(
        "SELECT EXISTS(
            SELECT 1 FROM agent_attachments WHERE repository_id = $1
         )",
    )
    .bind(repository.id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("instance provenance retention check");
    if retains_instance_history {
        // Reusable attachment/release provenance is deliberately permanent;
        // this random fixture cannot be deleted without violating that model.
        return;
    }
    sqlx::query(
        "DELETE FROM outbox WHERE aggregate_type = 'forge'
         AND (
           aggregate_id IN (SELECT id FROM run_requests WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM agent_config_revisions WHERE repository_id = $1)
           OR aggregate_id IN (SELECT id FROM git_receives WHERE repository_id = $1)
         )",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete forge outbox");
    sqlx::query("DELETE FROM run_requests WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run requests");
    sqlx::query("DELETE FROM agent_config_revisions WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete config revisions");
    sqlx::query("DELETE FROM git_refs WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete current refs");
    sqlx::query(
        "DELETE FROM git_ref_updates
         WHERE receive_id IN (SELECT id FROM git_receives WHERE repository_id = $1)",
    )
    .bind(repository.id.as_uuid())
    .execute(pool)
    .await
    .expect("delete ref updates");
    sqlx::query("DELETE FROM git_receives WHERE repository_id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete receives");
    sqlx::query("DELETE FROM repositories WHERE id = $1")
        .bind(repository.id.as_uuid())
        .execute(pool)
        .await
        .expect("delete repository");
    sqlx::query("DELETE FROM projects WHERE id = $1")
        .bind(repository.project_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete project");
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn git_output(directory: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .await
        .expect("run Git");
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 Git output")
        .trim()
        .to_owned()
}

const fn valid_config() -> &'static str {
    r#"
version = 2
[agent]
name = "Reviewer"
key = "reviewer"
[build]
command = "/bin/build"
working_directory = "/source"
root_image = "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 512
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/reviewer"
kind = "executable"
[guest]
command = "bin/reviewer"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 256
[root_image]
reference = "runtime@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[network]
profile = "disabled"
[triggers]
push = false
"#
}
