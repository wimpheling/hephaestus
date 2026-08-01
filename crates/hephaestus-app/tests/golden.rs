//! Opt-in daemon-level golden path through every production boundary.

use async_trait::async_trait;
use forge_domain::{GitRef, OrganizationId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use hephaestus_app::{AppConfig, HephaestusApp, OidcConfig, RunEventKind, VmBackendConfig};
use identity_domain::UserId;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use run_runtime_local::LocalRunRuntimeConfig;
use secret_broker::DenyingBrokerAdapter;
use secret_runtime::EphemeralSecretConfig;
use secret_store::LocalKeyProvider;
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use time::OffsetDateTime;
use tokio::process::Command;
use tokio::sync::{broadcast, watch};
use vm_libkrun::LibkrunConfig;
use vm_trait::{
    RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmId, VmInstance, VmProvider, VmSpec,
};
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

const ISSUER: &str = "https://issuer.golden.invalid";
const AUDIENCE: &str = "hephaestus-git";
const SIGNING_SECRET: &[u8] = b"golden-test-signing-secret-with-sufficient-entropy";
const ROOT_IMAGE: &str =
    "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const GOLDEN_AGENT: &str = r#"#!/bin/sh
  set -eu
  test -r /workspace/repo/input.txt
  test -r /run/hephaestus/parameters.json
  test -r /run/hephaestus/context.json
  if printf 'forbidden\n' > /release/write-must-fail 2>/dev/null; then
      exit 91
  fi
  if printf 'forbidden\n' > /workspace/repo/write-must-fail 2>/dev/null; then
      exit 92
  fi
  if printf 'forbidden\n' > /run/hephaestus/write-must-fail 2>/dev/null; then
      exit 93
  fi
  printf 'state-ok\n' > /var/lib/hephaestus/golden-state
  test "$(cat /var/lib/hephaestus/golden-state)" = "state-ok"
  printf 'agent edit\n' > /workspace/work/input.txt
  printf 'durable report\n' > /workspace/work/reports/result.txt
  "#;

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn bearer_push_starts_run_through_production_bootstrap() {
    drop(
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::try_from_default_env()
                    .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
            )
            .with_test_writer()
            .try_init(),
    );
    let (Ok(database_url), Ok(nats_url)) = (
        std::env::var("HEPHAESTUS_POSTGRES_TEST_URL"),
        std::env::var("HEPHAESTUS_NATS_TEST_URL"),
    ) else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("connect golden PostgreSQL");
    sqlx::migrate!("../../migrations")
        .run(&pool)
        .await
        .expect("apply application migrations");

    let temporary = tempfile::tempdir().expect("golden temporary root");
    let root = temporary.path().canonicalize().expect("canonical root");
    let repository_root = root.join("repositories");
    let storage = Arc::new(
        GitStorage::initialize(&repository_root)
            .await
            .expect("fixture Git storage"),
    );
    let fixture_repository = PgForgeRepository::new(pool.clone(), Arc::clone(&storage));
    let user_id = UserId::new();
    let organization_id = OrganizationId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Golden User')")
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization_id.as_uuid())
        .bind(format!("golden-{organization_id}"))
        .execute(&pool)
        .await
        .expect("seed organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
           VALUES ($1, $2, 'owner')",
    )
    .bind(organization_id.as_uuid())
    .bind(user_id.as_uuid())
    .execute(&pool)
    .await
    .expect("seed organization owner");
    sqlx::query(
        "INSERT INTO external_identities
           (user_id, issuer, subject, provider_metadata)
           VALUES ($1, $2, 'golden-subject', '{}')",
    )
    .bind(user_id.as_uuid())
    .bind(ISSUER)
    .execute(&pool)
    .await
    .expect("seed external identity");
    let project = fixture_repository
        .create_project_trusted(organization_id, "golden-project")
        .await
        .expect("seed project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project.id.as_uuid())
        .bind(user_id.as_uuid())
        .execute(&pool)
        .await
        .expect("seed project maintainer");
    let repository = fixture_repository
        .create_repository_trusted(&CreateRepository {
            project_id: project.id,
            name: String::from("golden-repository"),
            default_branch: GitRef::parse("refs/heads/main").expect("default ref"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("seed repository");
    seed_reusable_instance(
        &pool,
        user_id,
        project.id.as_uuid(),
        repository.id.as_uuid(),
        &root.join("release-artifacts"),
    )
    .await;

    let backend_fixture = backend_fixture(&root).await;
    let mut transient_runtime_roots = backend_fixture.transient_runtime_roots;
    transient_runtime_roots.push(root.join("workspaces"));
    let backend = git_backend().await;
    let secret_mount_root = root.join("secret-mounts");
    std::fs::create_dir(&secret_mount_root).expect("secret mount root");
    std::fs::set_permissions(
        &secret_mount_root,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("secret mount root mode");
    let app = HephaestusApp::build(AppConfig {
        database_url,
        nats_url: nats_url.clone(),
        http_listen: "127.0.0.1:0".parse().expect("ephemeral listen address"),
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token",
        ),
        repository_root: repository_root.clone(),
        git_http_backend: backend,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: String::from(ISSUER),
            audience: String::from(AUDIENCE),
            algorithm: Algorithm::HS256,
            decoding_key: jsonwebtoken::DecodingKey::from_secret(SIGNING_SECRET),
        },
        volumes: LocalVolumeConfig {
            volume_root: backend_fixture.volume_root,
            transient_runtime_roots,
            host_id: String::from("golden-host"),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: PathBuf::from("/usr/bin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root: root.join("workspaces"),
            artifact_root: root.join("artifacts"),
            repository_root: repository_root.clone(),
            git_binary: git_binary().await,
            limits: WorkspaceLimits::default(),
        },
        run_runtime: LocalRunRuntimeConfig {
            runtime_root: root.join("run-runtime"),
            release_artifact_root: root.join("release-artifacts"),
        },
        build_workspace_root: root.join("isolated-builds"),
        build_timeout: Duration::from_secs(30),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: false,
        },
        secret_keys: LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
            .expect("secret key"),
        secret_broker_socket: root.join("secret-broker.sock"),
        secret_broker_adapter: Arc::new(DenyingBrokerAdapter),
        vm_backend: backend_fixture.backend,
        root_images: BTreeMap::from([(
            String::from(ROOT_IMAGE),
            RootFilesystem::Directory {
                host_path: backend_fixture.root_image,
            },
        )]),
        runtime_policy: hephaestus_app::RuntimePolicy {
            version: String::from("golden/v1"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: true,
        },
        agent_state_capacity_bytes: 16 * 1024 * 1024,
        worker_concurrency: 4,
        outbox_poll_interval: Duration::from_millis(10),
        outbox_batch_size: 20,
        startup_timeout: Duration::from_secs(10),
        shutdown_timeout: Duration::from_secs(10),
    })
    .await
    .expect("build production application");
    let running = app.start().await.expect("start ready application");

    let token = signed_token();
    let source = root.join("source");
    tokio::fs::create_dir(&source)
        .await
        .expect("source repository");
    git(&source, &["init", "--initial-branch=main"]).await;
    git(&source, &["config", "user.name", "Golden Test"]).await;
    git(&source, &["config", "user.email", "golden@example.invalid"]).await;
    tokio::fs::write(source.join("agent.toml"), agent_config())
        .await
        .expect("provider-neutral agent.toml");
    tokio::fs::write(source.join("golden-agent.sh"), GOLDEN_AGENT)
        .await
        .expect("golden agent executable source");
    tokio::fs::write(source.join("input.txt"), "accepted\n")
        .await
        .expect("input file");
    tokio::fs::create_dir(source.join("reports"))
        .await
        .expect("reports directory");
    tokio::fs::write(source.join("reports/result.txt"), "initial\n")
        .await
        .expect("initial report");
    git(&source, &["add", "."]).await;
    git(&source, &["commit", "-m", "golden agent"]).await;
    let input_commit = git_output(&source, &["rev-parse", "HEAD"]).await;
    let remote = format!("http://{}/{}", running.http_addr(), repository.id);
    git(&source, &["remote", "add", "origin", &remote]).await;
    authenticated_git(&source, &token, &["push", "origin", "HEAD:refs/heads/main"]).await;

    let run_id: uuid::Uuid =
        sqlx::query_scalar("SELECT run_id FROM run_requests WHERE repository_id = $1")
            .bind(repository.id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("durable run request");
    let run_id = runtime_types::RunId::from_uuid(run_id);
    let result_wait = running
        .wait_for_run_event(
            run_id,
            RunEventKind::ResultCompleted,
            Duration::from_secs(10),
        )
        .await;
    if result_wait.is_err() {
        diagnose_golden_timeout(&pool, repository.id.as_uuid(), run_id).await;
    }
    result_wait.expect("persisted result completion");

    let (result_ref, result_commit): (String, String) = sqlx::query_as(
        "SELECT result_ref, result_commit
           FROM run_results WHERE run_id = $1 AND state = 'completed'",
    )
    .bind(run_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("completed result");
    let bare = root
        .join("repositories")
        .join(format!("{}.git", repository.id));
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &result_ref]).await,
        result_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["rev-parse", &format!("{result_commit}^")]).await,
        input_commit
    );
    assert_eq!(
        git_output_bare(&bare, &["show", &format!("{result_commit}:input.txt")]).await,
        "agent edit"
    );

    let permissions: Vec<String> = sqlx::query_scalar(
        "SELECT permission FROM authorization_audit_events
           WHERE actor_id = $1
           ORDER BY permission",
    )
    .bind(user_id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("authorization audit");
    assert!(permissions.iter().any(|value| value == "can_write"));
    assert!(permissions.iter().any(|value| value == "can_execute"));
    assert!(permissions.iter().any(|value| value == "can_use"));
    let mapped_profile: bool = sqlx::query_scalar(
        "SELECT EXISTS(
              SELECT 1 FROM user_profiles
              WHERE user_id = $1
                AND validated_claims->>'sub' = 'golden-subject'
           )",
    )
    .bind(user_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("OIDC profile mapping");
    assert!(mapped_profile);

    running.shutdown().await.expect("graceful daemon shutdown");
    let (signals, unpublished_signals): (i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE message_class = 'internal_signal'),
                  count(*) FILTER (
                      WHERE message_class = 'internal_signal' AND published_at IS NULL
                  )
           FROM outbox",
    )
    .fetch_one(&pool)
    .await
    .expect("internal signal outbox census");
    assert_eq!(
        signals, 0,
        "legacy informational signals must not be emitted"
    );
    assert_eq!(
        unpublished_signals, 0,
        "legacy informational signals must never remain pending"
    );
    cleanup_streams(&nats_url).await;
}

fn signed_token() -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": ISSUER,
            "sub": "golden-subject",
            "aud": AUDIENCE,
            "iat": now,
            "exp": now + 300,
            "email": "golden@example.invalid",
            "email_verified": true
        }),
        &EncodingKey::from_secret(SIGNING_SECRET),
    )
    .expect("sign golden bearer token")
}

const fn agent_config() -> &'static str {
    r#"
  version = 2
  [agent]
  name = "Golden Agent"
  key = "golden-agent"
  [build]
  command = "/bin/sh"
  arguments = ["-c", "mkdir -p /workspace/output/bin && cp /workspace/source/golden-agent.sh /workspace/output/bin/golden && chmod 0555 /workspace/output/bin/golden"]
  working_directory = "/workspace/source"
  root_image = "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
  triggers = ["refs/heads/main"]
  [build.resources]
  vcpus = 1
  memory_mib = 128
  [build.network]
  profile = "disabled"
  [[build.artifacts]]
  path = "bin/golden"
  kind = "executable"
  [guest]
  command = "bin/golden"
  arguments = []
  working_directory = "bin"
  [resources]
  vcpus = 1
  memory_mib = 128
  [root_image]
  reference = "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
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
  refs = ["refs/heads/main"]
  [update_hook]
  command = "bin/golden"
  arguments = []
  timeout_seconds = 60
  [update_hook.resources]
  vcpus = 1
  memory_mib = 128
  "#
}

#[allow(clippy::too_many_lines)]
async fn seed_reusable_instance(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    artifact_root: &Path,
) {
    let build_id = uuid::Uuid::new_v4();
    let family_id = uuid::Uuid::new_v4();
    let release_id = uuid::Uuid::new_v4();
    let release_agent_id = uuid::Uuid::new_v4();
    let instance_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let attachment_id = uuid::Uuid::new_v4();
    let state_volume_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let artifact = GOLDEN_AGENT.as_bytes();
    let release_configuration = serde_json::to_value(
        agent_config::parse(agent_config().as_bytes())
            .config
            .expect("golden reusable configuration should parse"),
    )
    .expect("serialize golden reusable configuration");
    tokio::fs::create_dir_all(artifact_root)
        .await
        .expect("release artifact root");
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();

    sqlx::query(
        "INSERT INTO build_requests
           (id, repository_id, source_commit, source_ref,
            build_definition_hash, state, created_by, completed_at)
           VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded',
                   $5, now())",
    )
    .bind(build_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind([1_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable build");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
           VALUES ($1, $2, 'golden-agent')",
    )
    .bind(family_id)
    .bind(repository_id)
    .execute(pool)
    .await
    .expect("seed reusable family");
    sqlx::query(
        "INSERT INTO releases
           (id, repository_id, version, source_commit, source_ref,
           build_request_id, build_definition_hash, configuration,
            configuration_hash, manifest_hash, state, published_at)
           VALUES ($1, $2, 'v1', $3, 'refs/heads/main', $4, $5,
                   $6, $7, $8, 'published', now())",
    )
    .bind(release_id)
    .bind(repository_id)
    .bind("a".repeat(40))
    .bind(build_id)
    .bind([1_u8; 32].as_slice())
    .bind(release_configuration)
    .bind([2_u8; 32].as_slice())
    .bind([3_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed reusable release");
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
           VALUES ($1, $2, 'bin/golden', 'executable', 365, $3, $4,
                   'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(release_id)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed reusable artifact");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
           VALUES ($1, $2, $3, 'golden-agent', 'Golden Agent', $4, $5,
                   '[]', '[]', true, $6)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/golden",
        "arguments": [],
        "working_directory": "bin",
        "root_image_digest": ROOT_IMAGE,
        "requires_state": true
    }))
    .bind([4_u8; 32].as_slice())
    .bind(serde_json::json!({
        "command": "bin/golden",
        "arguments": [],
        "timeout_seconds": 60,
        "resources": {"vcpus": 1, "memory_mib": 128}
    }))
    .execute(pool)
    .await
    .expect("seed reusable release agent");
    sqlx::query(
        "INSERT INTO agent_instances
           (id, project_id, family_id, name, state, created_by)
           VALUES ($1, $2, $3, 'golden-agent', 'active', $4)",
    )
    .bind(instance_id)
    .bind(project_id)
    .bind(family_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable instance");
    sqlx::query(
        "INSERT INTO agent_instance_state_volumes
           (id, instance_id, state, capacity_bytes)
           VALUES ($1, $2, 'uninitialized', $3)",
    )
    .bind(state_volume_id)
    .bind(instance_id)
    .bind(16_i64 * 1024 * 1024)
    .execute(pool)
    .await
    .expect("seed reusable instance state volume");
    sqlx::query("UPDATE agent_instances SET state_volume_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(state_volume_id)
        .execute(pool)
        .await
        .expect("attach reusable instance state volume");
    sqlx::query(
        "INSERT INTO agent_instance_revisions
           (id, instance_id, release_agent_id, parameters, parameter_hash,
            secret_bindings, resource_selection, network_restriction,
            effective_runtime_policy, effective_policy_hash,
            platform_policy_version, runnable, diagnostics, created_by)
           VALUES ($1, $2, $3, '{}', $4, '[]', $5, $6, $5, $7,
                   'platform/v1', true, '[]', $8)",
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
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable revision");
    sqlx::query("UPDATE agent_instances SET active_revision_id = $2 WHERE id = $1")
        .bind(instance_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate reusable revision");
    sqlx::query(
        "INSERT INTO agent_attachments
           (id, instance_id, project_id, repository_id, ref_selector,
            trigger_policy, enabled, created_by)
           VALUES ($1, $2, $3, $4, 'refs/heads/main', 'push', true, $5)",
    )
    .bind(attachment_id)
    .bind(instance_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed reusable attachment");
}

/// Emits bounded, payload-free state when the production golden run times out.
/// This is intentionally test-only: it helps distinguish an un-dispatched
/// command from a persisted guest failure without exposing guest logs or
/// provider error strings (which could contain paths or secret material).
// The diagnostic deliberately keeps each bounded status query together so a
// timeout report is emitted atomically from this test-only helper.
#[allow(clippy::too_many_lines, clippy::type_complexity)]
async fn diagnose_golden_timeout(
    pool: &sqlx::PgPool,
    repository_id: uuid::Uuid,
    run_id: runtime_types::RunId,
) {
    let run = sqlx::query(
        "SELECT state, outcome, exit_code, exit_signal,
                vm_id IS NOT NULL AS has_vm
           FROM runs WHERE id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match run {
        Ok(Some(row)) => {
            let state: String = row.get("state");
            let outcome: Option<String> = row.get("outcome");
            let exit_code: Option<i32> = row.get("exit_code");
            let exit_signal: Option<i32> = row.get("exit_signal");
            let has_vm: bool = row.get("has_vm");
            eprintln!(
                "golden timeout: run state={state} outcome={outcome:?} exit_code={exit_code:?} exit_signal={exit_signal:?} has_vm={has_vm}"
            );
        }
        Ok(None) => eprintln!("golden timeout: run row missing"),
        Err(error) => eprintln!("golden timeout: run query failed: {error}"),
    }

    let request = sqlx::query(
        "SELECT request_kind, dispatch_state
           FROM run_requests WHERE run_id = $1",
    )
    .bind(run_id.as_uuid())
    .fetch_optional(pool)
    .await;
    match request {
        Ok(Some(row)) => {
            let kind: String = row.get("request_kind");
            let dispatch_state: String = row.get("dispatch_state");
            eprintln!("golden timeout: run request kind={kind} dispatch_state={dispatch_state}");
        }
        Ok(None) => eprintln!("golden timeout: run request row missing"),
        Err(error) => eprintln!("golden timeout: run request query failed: {error}"),
    }

    match sqlx::query_scalar::<_, String>(
        "SELECT event_type FROM run_events WHERE run_id = $1 ORDER BY sequence",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(events) => eprintln!("golden timeout: run events={events:?}"),
        Err(error) => eprintln!("golden timeout: run events query failed: {error}"),
    }

    match sqlx::query(
        "SELECT subject, published_at IS NOT NULL AS published
           FROM outbox
          WHERE aggregate_id = $1
          ORDER BY occurred_at, id",
    )
    .bind(run_id.as_uuid())
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(String, bool)> = rows
                .into_iter()
                .map(|row| (row.get("subject"), row.get("published")))
                .collect();
            eprintln!("golden timeout: run outbox={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: run outbox query failed: {error}"),
    }

    match sqlx::query(
        "SELECT request.state AS request_state,
                execution.state AS execution_state,
                execution.exit_code, execution.exit_signal,
                execution.failure_code
           FROM build_requests AS request
           LEFT JOIN build_executions AS execution
             ON execution.build_request_id = request.id
          WHERE request.repository_id = $1
          ORDER BY request.created_at DESC, request.id DESC
          LIMIT 3",
    )
    .bind(repository_id)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => {
            let entries: Vec<(
                String,
                Option<String>,
                Option<i32>,
                Option<i32>,
                Option<String>,
            )> = rows
                .into_iter()
                .map(|row| {
                    (
                        row.get("request_state"),
                        row.get("execution_state"),
                        row.get("exit_code"),
                        row.get("exit_signal"),
                        row.get("failure_code"),
                    )
                })
                .collect();
            eprintln!("golden timeout: recent builds={entries:?}");
        }
        Err(error) => eprintln!("golden timeout: build query failed: {error}"),
    }
}

async fn git_backend() -> PathBuf {
    let exec_path = git_output(Path::new("."), &["--exec-path"]).await;
    PathBuf::from(exec_path).join("git-http-backend")
}

async fn git_binary() -> PathBuf {
    let output = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .await
        .expect("resolve Git binary");
    assert!(output.status.success());
    PathBuf::from(
        String::from_utf8(output.stdout)
            .expect("UTF-8 Git path")
            .trim(),
    )
}

struct BackendFixture {
    backend: VmBackendConfig,
    root_image: PathBuf,
    volume_root: PathBuf,
    transient_runtime_roots: Vec<PathBuf>,
}

async fn backend_fixture(temporary_root: &Path) -> BackendFixture {
    if env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1") {
        let runtime_root = required_path("HEPHAESTUS_LIBKRUN_RUNTIME_ROOT");
        let image_root = required_path("HEPHAESTUS_LIBKRUN_IMAGE_ROOT");
        let root_image = required_path("HEPHAESTUS_LIBKRUN_ROOTFS");
        let disk_root = required_path("HEPHAESTUS_LIBKRUN_DISK_ROOT");
        let mount_root = required_path("HEPHAESTUS_LIBKRUN_MOUNT_ROOT");
        let worker = required_path("HEPHAESTUS_LIBKRUN_WORKER");
        let cgroup_root = required_path("HEPHAESTUS_LIBKRUN_CGROUP_ROOT");
        let volume_root = disk_root.join(format!("app-golden-volumes-{}", uuid::Uuid::new_v4()));
        let provider = LibkrunConfig::new(
            runtime_root.clone(),
            vec![image_root],
            vec![disk_root],
            vec![mount_root, temporary_root.to_path_buf()],
            worker,
            cgroup_root,
        );
        BackendFixture {
            backend: VmBackendConfig::Libkrun(Box::new(provider)),
            root_image,
            volume_root,
            transient_runtime_roots: vec![runtime_root],
        }
    } else {
        let root_image = temporary_root.join("root-image");
        tokio::fs::create_dir(&root_image)
            .await
            .expect("root image directory");
        BackendFixture {
            backend: VmBackendConfig::Custom(Arc::new(ResultGuestProvider)),
            root_image,
            volume_root: temporary_root.join("volumes"),
            transient_runtime_roots: Vec::new(),
        }
    }
}

struct ResultGuestProvider;

#[async_trait]
impl VmProvider for ResultGuestProvider {
    fn name(&self) -> &'static str {
        "golden-result-guest"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        Ok(Arc::new(ResultGuestInstance::new(spec)?))
    }

    async fn cleanup_orphan(&self, _id: &VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct ResultGuestInstance {
    id: VmId,
    work: PathBuf,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl ResultGuestInstance {
    fn new(spec: VmSpec) -> Result<Self, VmError> {
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-source")
            .ok_or_else(|| VmError::InvalidSpec {
                field: String::from("mounts"),
                reason: String::from("repository source mount is missing"),
            })?;
        if !source.read_only {
            return Err(VmError::InvalidSpec {
                field: String::from("mounts"),
                reason: String::from("repository source mount is writable"),
            });
        }
        let work = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "repository-work")
            .ok_or_else(|| VmError::InvalidSpec {
                field: String::from("mounts"),
                reason: String::from("repository work mount is missing"),
            })?;
        if work.read_only {
            return Err(VmError::InvalidSpec {
                field: String::from("mounts"),
                reason: String::from("repository work mount is read-only"),
            });
        }
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Ok(Self {
            id: spec.id,
            work: work.host_path.clone(),
            events,
            exit,
        })
    }
}

#[async_trait]
impl VmInstance for ResultGuestInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        let _started = self.events.send(VmEvent::Started {
            ingress: Vec::new(),
        });
        let _ready = self.events.send(VmEvent::Ready);
        tokio::fs::write(self.work.join("input.txt"), "agent edit\n")
            .await
            .map_err(test_vm_error)?;
        tokio::fs::write(self.work.join("reports/result.txt"), "durable report\n")
            .await
            .map_err(test_vm_error)?;
        let _finalize = self.events.send(VmEvent::FinalizeResult {
            message: String::from("golden agent result"),
        });
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        let _exited = self.events.send(VmEvent::Exited(exit.clone()));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow_and_update().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver
                .changed()
                .await
                .map_err(|_| VmError::InvalidState("golden result guest exited"))?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn test_vm_error(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("golden-result-guest"),
        code: String::from("workspace-write"),
        source: Box::new(error),
    }
}

fn required_path(name: &str) -> PathBuf {
    env::var_os(name).map_or_else(
        || panic!("{name} is required for libkrun golden E2E"),
        PathBuf::from,
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
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

async fn authenticated_git(directory: &Path, token: &str, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "http.extraHeader")
        .env(
            "GIT_CONFIG_VALUE_0",
            format!("Authorization: Bearer {token}"),
        )
        .output()
        .await
        .expect("run authenticated Git");
    assert!(
        output.status.success(),
        "authenticated Git failed: {}",
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
    assert!(output.status.success());
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
        "bare Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("UTF-8 bare Git output")
        .trim()
        .to_owned()
}

async fn cleanup_streams(nats_url: &str) {
    let Ok(client) = async_nats::connect(nats_url).await else {
        return;
    };
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        drop(context.delete_stream(stream).await);
    }
}
