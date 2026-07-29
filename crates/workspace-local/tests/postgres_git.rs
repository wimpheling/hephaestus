//! Opt-in exact-commit and controlled-result integration coverage.

use forge_domain::{CommitSha, GitRef, OrganizationId, ReceiveId, RefUpdate};
use forge_service::{CreateRepository, GitStorage, PgForgeRepository};
use run_domain::StartRun;
use run_orchestrator::{PgRunRepository, RunRepository};
use serial_test::serial;
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::process::Command;
use uuid::Uuid;
use workspace_domain::RunWorkspaceManager;
use workspace_local::{LocalWorkspaceConfig, LocalWorkspaceManager, WorkspaceLimits};

#[tokio::test]
#[serial]
#[allow(clippy::too_many_lines)]
async fn exact_commit_becomes_one_controlled_result_with_durable_artifacts() {
    let Some(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL").ok() else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply migrations");
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let repository_root = root.join("repositories");
    let storage = Arc::new(
        GitStorage::initialize(&repository_root)
            .await
            .expect("initialize repositories"),
    );
    let forge = PgForgeRepository::new(pool.clone(), storage);
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind(format!("workspace-{organization_id}"))
        .execute(&pool)
        .await
        .expect("organization");
    let project = forge
        .create_project_trusted(organization_id, "workspace-project")
        .await
        .expect("project");
    let repository = forge
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("workspace-repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default branch"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("repository");

    let work = root.join("input");
    tokio::fs::create_dir(&work).await.expect("input work tree");
    git(&work, &["init", "--initial-branch=main"]).await;
    git(&work, &["config", "user.name", "Workspace Test"]).await;
    git(
        &work,
        &["config", "user.email", "workspace@example.invalid"],
    )
    .await;
    tokio::fs::write(work.join("agent.toml"), agent_config())
        .await
        .expect("agent config");
    tokio::fs::write(work.join("input.txt"), "accepted\n")
        .await
        .expect("input file");
    tokio::fs::create_dir(work.join("reports"))
        .await
        .expect("report directory");
    tokio::fs::write(work.join("reports/result.txt"), "initial\n")
        .await
        .expect("initial declared file");
    git(&work, &["add", "."]).await;
    git(&work, &["commit", "-m", "accepted input"]).await;
    let accepted =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("accepted commit");
    seed_attached_instance(&pool, project.id, repository.id, accepted.as_str()).await;
    let bare = repository_root.join(format!("{}.git", repository.id));
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;
    let receive = forge
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "workspace-test",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("main ref"),
                old_commit: None,
                new_commit: Some(accepted.clone()),
            }],
        )
        .await
        .expect("accept receive");
    let request = receive.run_requests.first().expect("one run request");
    let command: StartRun = request.command.clone();
    let runs = PgRunRepository::new(pool.clone());
    let run = runs
        .create_run(&command)
        .await
        .expect("create durable run")
        .run;

    tokio::fs::write(work.join("input.txt"), "newer branch tip\n")
        .await
        .expect("newer input");
    git(&work, &["add", "input.txt"]).await;
    git(&work, &["commit", "-m", "move main"]).await;
    let newer =
        CommitSha::parse(git_output(&work, &["rev-parse", "HEAD"]).await).expect("newer commit");
    git(
        &work,
        &[
            "push",
            bare.to_str().expect("UTF-8 bare path"),
            "HEAD:refs/heads/main",
        ],
    )
    .await;

    let mut manager = LocalWorkspaceManager::new(
        pool.clone(),
        LocalWorkspaceConfig {
            workspace_root: root.join("workspaces"),
            artifact_root: root.join("artifacts"),
            repository_root: repository_root.clone(),
            git_binary: git_binary().await,
            limits: WorkspaceLimits::default(),
        },
    )
    .expect("workspace configuration");
    manager.initialize().expect("workspace roots");
    let prepared = manager.prepare(&run).await.expect("prepare exact commit");
    assert_eq!(prepared.mounts.len(), 2);
    let source = prepared
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-source")
        .expect("source mount");
    let writable = prepared
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-work")
        .expect("work mount");
    assert!(source.read_only);
    assert!(!writable.read_only);
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect("materialized source"),
        "accepted\n"
    );
    tokio::fs::write(writable.host_path.join("input.txt"), "agent edit\n")
        .await
        .expect("edit writable workspace");
    tokio::fs::write(
        writable.host_path.join("reports/result.txt"),
        "durable report\n",
    )
    .await
    .expect("edit declared artifact");
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect("source remains readable"),
        "accepted\n",
        "writable workspace mutation changed the immutable source tree"
    );

    let published = manager
        .finalize(&run, "agent result")
        .await
        .expect("publish controlled result")
        .expect("workspace result");
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &published.result_ref]).await,
        published.result_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", "refs/heads/main"]).await,
        newer.as_str()
    );
    assert_eq!(
        git_output_bare(
            &bare,
            &["rev-parse", &format!("{}^", published.result_commit)]
        )
        .await,
        accepted.as_str()
    );
    assert_eq!(
        git_output_bare(
            &bare,
            &["show", &format!("{}:input.txt", published.result_commit)]
        )
        .await,
        "agent edit"
    );
    assert_eq!(
        tokio::fs::read_to_string(source.host_path.join("input.txt"))
            .await
            .expect_err("sealed source path was cleaned")
            .kind(),
        std::io::ErrorKind::NotFound
    );
    let artifact_kinds: Vec<String> = sqlx::query_scalar(
        "SELECT kind FROM result_artifacts artifact
         JOIN run_results result ON result.id = artifact.result_id
         WHERE result.run_id = $1 ORDER BY kind",
    )
    .bind(run.id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("result artifacts");
    assert_eq!(
        artifact_kinds,
        ["declared_file", "exit", "logs", "manifest", "patch"]
    );
    let manifest_key: String = sqlx::query_scalar(
        "SELECT artifact.storage_key
         FROM result_artifacts artifact
         JOIN run_results result ON result.id = artifact.result_id
         WHERE result.run_id = $1 AND artifact.kind = 'manifest'",
    )
    .bind(run.id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("manifest storage key");
    let manifest: serde_json::Value = serde_json::from_slice(
        &tokio::fs::read(root.join("artifacts").join(manifest_key))
            .await
            .expect("durable manifest"),
    )
    .expect("manifest JSON");
    assert_eq!(
        manifest["result_commit"].as_str(),
        Some(published.result_commit.as_str())
    );
    assert_eq!(manifest["input_commit"].as_str(), Some(accepted.as_str()));
    let result_state: String =
        sqlx::query_scalar("SELECT state FROM run_results WHERE run_id = $1")
            .bind(run.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("result state");
    assert_eq!(result_state, "completed");
    let duplicate = manager
        .finalize(&run, "ignored retry message")
        .await
        .expect("idempotent finalize")
        .expect("existing result");
    assert_eq!(duplicate, published);
    git(&bare, &["update-ref", "-d", &published.result_ref]).await;
    sqlx::query(
        "UPDATE run_results
         SET state = 'prepared', published_at = NULL, completed_at = NULL
         WHERE run_id = $1",
    )
    .bind(run.id.as_uuid())
    .execute(&pool)
    .await
    .expect("simulate crash before ref publication");
    assert_eq!(manager.recover().await.expect("recover result CAS"), 1);
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &published.result_ref]).await,
        published.result_commit
    );

    let second_receive = forge
        .accept_receive(
            &repository,
            ReceiveId::new(),
            "workspace-test",
            &[RefUpdate {
                git_ref: GitRef::parse("refs/heads/main").expect("main ref"),
                old_commit: Some(accepted),
                new_commit: Some(newer),
            }],
        )
        .await
        .expect("accept second receive");
    let second_command = second_receive
        .run_requests
        .first()
        .expect("second run request")
        .command
        .clone();
    let second_run = runs
        .create_run(&second_command)
        .await
        .expect("create second run")
        .run;
    let second_workspace = manager
        .prepare(&second_run)
        .await
        .expect("prepare second exact commit");
    let second_source = second_workspace
        .mounts
        .iter()
        .find(|mount| mount.tag == "repository-source")
        .expect("second source mount");
    assert_eq!(
        tokio::fs::read_to_string(second_source.host_path.join("input.txt"))
            .await
            .expect("second materialized source"),
        "newer branch tip\n"
    );
    let socket = UnixListener::bind(
        second_workspace
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work")
            .expect("second writable mount")
            .host_path
            .join("guest.sock"),
    )
    .expect("guest-created socket");
    drop(socket);
    assert!(
        manager
            .finalize(&second_run, "unsafe result")
            .await
            .is_err(),
        "a guest-created socket was imported"
    );
    let rejected_ref: String =
        sqlx::query_scalar("SELECT result_ref FROM run_results WHERE run_id = $1")
            .bind(second_run.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("rejected result ref");
    assert!(
        !git_ref_exists(&bare, &rejected_ref).await,
        "a rejected result ref was published"
    );
    manager
        .abandon(second_run.id)
        .await
        .expect("clean second ephemeral workspace");
    assert!(
        root.join("artifacts").join(run.id.to_string()).is_dir(),
        "cleaning a second workspace removed the first run's durable artifacts"
    );
}

// The exact result-provenance contract needs the whole immutable graph; one
// fixture function keeps every foreign-key edge visible beside its test.
#[allow(clippy::too_many_lines)]
async fn seed_attached_instance(
    pool: &PgPool,
    project_id: forge_domain::ProjectId,
    repository_id: forge_domain::RepositoryId,
    commit: &str,
) {
    let release_configuration = agent_config::parse(agent_config().as_bytes())
        .config
        .expect("fixture release configuration should parse");
    let build_id = Uuid::new_v4();
    let family_id = Uuid::new_v4();
    let release_id = Uuid::new_v4();
    let release_agent_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let mut tx = pool.begin().await.expect("begin exact instance fixture");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref,
          build_definition_hash, state, completed_at)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', now())",
    )
    .bind(build_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind([1_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'workspace-agent')",
    )
    .bind(family_id)
    .bind(repository_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact family");
    sqlx::query(
        "INSERT INTO releases
         (id, repository_id, version, source_commit, source_ref,
          build_request_id, build_definition_hash, configuration,
          configuration_hash, manifest_hash, state, published_at)
         VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                 $8, $6, $7, 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id.as_uuid())
    .bind(commit)
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .bind(serde_json::to_value(release_configuration).expect("serialize release configuration"))
    .execute(&mut *tx)
    .await
    .expect("seed exact release");
    sqlx::query(
        "INSERT INTO release_agents
         (id, release_id, family_id, agent_key, display_name,
          runtime_contract, runtime_contract_hash, parameter_schema,
          secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'workspace-agent', 'Workspace Agent',
                 $4, $5, '[]', '[]', false)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "command": "bin/agent",
        "arguments": [],
        "working_directory": ".",
        "root_image_digest": "fixture"
    }))
    .bind([4_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact release agent");
    sqlx::query(
        "INSERT INTO agent_instances
         (id, project_id, family_id, name, state)
         VALUES ($1, $2, $3, $4, 'active')",
    )
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(family_id)
    .bind(format!("workspace_{}", Uuid::new_v4().simple()))
    .execute(&mut *tx)
    .await
    .expect("seed exact instance");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
         (id, instance_id, release_agent_id, parameters, parameter_hash,
          resource_selection, network_restriction, effective_runtime_policy,
          effective_policy_hash, platform_policy_version, runnable, diagnostics)
         VALUES ($1, $2, $3, '{}', $4, $5, $6, $5, $7,
                 'platform/test', true, '[]')",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({
        "vcpus": 1,
        "memory_mib": 128,
        "network": "disabled"
    }))
    .bind(serde_json::json!({"network": "disabled"}))
    .bind([6_u8; 32].as_slice())
    .execute(&mut *tx)
    .await
    .expect("seed exact revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(&mut *tx)
        .await
        .expect("activate exact revision");
    sqlx::query(
        "INSERT INTO agent_attachments
         (id, instance_id, project_id, repository_id, ref_selector,
          trigger_policy, enabled)
         VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true)",
    )
    .bind(Uuid::new_v4())
    .bind(instance_id)
    .bind(project_id.as_uuid())
    .bind(repository_id.as_uuid())
    .execute(&mut *tx)
    .await
    .expect("seed exact attachment");
    tx.commit().await.expect("commit exact instance fixture");
}

const fn agent_config() -> &'static str {
    r#"
version = 2
[agent]
name = "workspace-agent"
key = "workspace-agent"
[build]
command = "/usr/bin/build"
working_directory = "/workspace/source"
root_image = "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
triggers = ["refs/heads/main"]
[build.resources]
vcpus = 1
memory_mib = 128
[build.network]
profile = "disabled"
[[build.artifacts]]
path = "bin/agent"
kind = "executable"
[guest]
command = "bin/agent"
arguments = []
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "image@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = false
[results]
declared_files = ["reports/result.txt"]
[network]
profile = "disabled"
[triggers]
push = true
refs = ["refs/heads/main"]
"#
}

async fn git_binary() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .await
        .expect("resolve Git executable");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git executable")
            .trim(),
    )
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

async fn git_output_bare(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(arguments)
        .output()
        .await
        .expect("run bare Git");
    assert!(
        output.status.success(),
        "bare git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 bare Git output")
        .trim()
        .to_owned()
}

async fn git_ref_exists(repository: &Path, git_ref: &str) -> bool {
    Command::new("git")
        .arg(format!("--git-dir={}", repository.display()))
        .args(["rev-parse", "--verify", git_ref])
        .output()
        .await
        .expect("inspect bare Git ref")
        .status
        .success()
}
