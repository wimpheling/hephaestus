//! Opt-in `PostgreSQL` and `JetStream` receive-processing coverage.

use forge_domain::{CommitSha, GitRef, ReceiveId, RefUpdate, Repository};
use forge_service::{
    CreateRepository, ForgeNatsOutboxPublisher, GitStorage, PgForgeRepository, RUN_START_SUBJECT,
    ensure_forge_jetstream_topology,
};
use futures_util::StreamExt;
use run_domain::{CancelRun, Run, RunState, StartRun};
use run_orchestrator::{
    NatsCommandHandler, PgRunRepository, RunOrchestrator, RunRepository, VmSpecFactory,
    ensure_jetstream_topology,
};
use runtime_types::{CommandId, RunId};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{collections::BTreeMap, path::Path, sync::Arc, time::Duration};
use tokio::process::Command;
use vm_fake::FakeProvider;
use vm_trait::{GuestCommand, NetworkMode, RootFilesystem, VmError, VmId, VmResources, VmSpec};
use volume_local::{LocalVolumeConfig, LocalVolumeStore};

#[tokio::test]
#[serial]
async fn persists_exact_config_and_deduplicates_receive() {
    let Some((pool, service, repository, temporary)) = fixture().await else {
        return;
    };
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
    .bind(RUN_START_SUBJECT)
    .bind(first.run_requests[0].id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("start command count");
    assert_eq!(stored_commit, commit.as_str());
    assert_eq!(starts, 1);

    let work = temporary.path().join("work");
    let invalid = valid_config().replace("version = 1", "version = 99");
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
    assert!(invalid_result.run_requests.is_empty());
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
    let (_, update) = commit_and_update(&temporary, &repository, valid_config()).await;
    service
        .accept_receive(&repository, ReceiveId::new(), "integration-user", &[update])
        .await
        .expect("accepted receive");

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
        2
    );
    assert_eq!(stream.info().await.expect("stream state").state.messages, 2);
    sqlx::query(
        "UPDATE outbox SET published_at = NULL
         WHERE aggregate_type = 'forge'
           AND aggregate_id IN (
             SELECT id FROM run_requests WHERE repository_id = $1
             UNION SELECT id FROM git_receives WHERE repository_id = $1
           )",
    )
    .bind(repository.id.as_uuid())
    .execute(&pool)
    .await
    .expect("simulate acknowledgement loss");
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("retry publication"),
        2
    );
    assert_eq!(
        stream
            .info()
            .await
            .expect("deduplicated state")
            .state
            .messages,
        2
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
    run_repository
        .initialize()
        .await
        .expect("run repository migrations");
    let volume_root = temporary.path().join("volumes");
    let volumes = Arc::new(
        LocalVolumeStore::new(
            pool.clone(),
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
    assert_eq!(
        publisher
            .publish_pending(&service, 10)
            .await
            .expect("publish receive and run command"),
        2
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

impl VmSpecFactory for TestSpecFactory {
    fn build(&self, run: &Run) -> Result<VmSpec, VmError> {
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
    sqlx::query("DELETE FROM runs WHERE id = $1")
        .bind(command.run_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete run");
    if let Some(volume_id) = volume_id {
        sqlx::query("DELETE FROM volume_leases WHERE volume_id = $1")
            .bind(volume_id)
            .execute(pool)
            .await
            .expect("delete volume leases");
        sqlx::query("DELETE FROM agent_state_volumes WHERE id = $1")
            .bind(volume_id)
            .execute(pool)
            .await
            .expect("delete volume");
    }
    sqlx::query("DELETE FROM agents WHERE id = $1")
        .bind(command.agent_id.as_uuid())
        .execute(pool)
        .await
        .expect("delete agent");
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
    let project = service
        .create_project("forge-service-integration")
        .await
        .expect("project");
    let repository = service
        .create_repository(&CreateRepository {
            project_id: project.id,
            name: String::from("repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");
    Some((pool, service, repository, temporary))
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
version = 1
[agent]
name = "reviewer"
[guest]
command = "/usr/bin/review"
arguments = []
working_directory = "/workspace"
[resources]
vcpus = 1
memory_mib = 256
[root_image]
reference = "image@sha256:abc"
[workspace]
mount = true
path = "/workspace"
read_only = true
[state_volume]
enabled = true
[network]
profile = "disabled"
[triggers]
push = true
refs = ["refs/heads/main"]
"#
}
