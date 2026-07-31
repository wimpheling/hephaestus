//! Opt-in real-PostgreSQL isolated build-to-draft integration.

use async_trait::async_trait;
use authz_postgres::PostgresMelangeAuthorizer;
use build_orchestrator::{BuildExecutionError, BuildExecutor, BuildExecutorConfig};
use build_postgres::PgBuildRepository;
use release_artifact_store::LocalArtifactStore;
use release_domain::{BuildRequestId, ReleaseAgentId, ReleaseArtifactId, ReleaseId};
use release_postgres::ReleaseService;
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::{broadcast, watch};
use uuid::Uuid;
use vm_trait::{
    LogStream, RootFilesystem, StopMode, VmError, VmEvent, VmExit, VmInstance, VmMetric,
    VmProvider, VmSpec,
};

const CONFIG: &str = r#"
version = 2
[agent]
name = "isolated-builder"
key = "isolated-builder"
[build]
command = "/usr/bin/fake-build"
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
media_type = "application/x-hephaestus-test"
[guest]
command = "bin/agent"
working_directory = "bin"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "run@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = false
[network]
profile = "disabled"
[triggers]
push = false
refs = []
"#;

#[tokio::test]
#[serial]
async fn exact_guest_output_becomes_one_immutable_draft() {
    let Ok(database_url) = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL") else {
        return;
    };
    let pool = PgPoolOptions::new()
        .max_connections(6)
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
    let workspace_root = root.join("builds");
    let artifact_root = root.join("release-artifacts");
    let root_image = root.join("root-image");
    for directory in [
        &repository_root,
        &workspace_root,
        &artifact_root,
        &root_image,
    ] {
        fs::create_dir(directory).expect("private root");
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700)).expect("private mode");
    }
    let (repository_id, commit) = source_repository(&root, &repository_root);
    let build_id = seed(&pool, repository_id, &commit).await;
    let provisions = Arc::new(AtomicUsize::new(0));
    let fail_next_provision = Arc::new(AtomicBool::new(false));
    let provider: Arc<dyn VmProvider> = Arc::new(OutputProvider {
        provisions: Arc::clone(&provisions),
        fail_next_provision: Arc::clone(&fail_next_provision),
    });
    let releases = Arc::new(ReleaseService::new(
        pool.clone(),
        Arc::new(PostgresMelangeAuthorizer),
    ));
    let artifact_store = LocalArtifactStore::new(artifact_root.clone()).expect("artifact store");
    let executor = BuildExecutor::initialize(
        Arc::new(PgBuildRepository::new(pool.clone())),
        provider,
        artifact_store.clone(),
        releases,
        BuildExecutorConfig {
            workspace_root,
            repository_root,
            git_binary: fs::canonicalize("/usr/bin/git").expect("Git binary"),
            root_images: BTreeMap::from([(
                String::from(
                    "build@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                ),
                RootFilesystem::Directory {
                    host_path: root_image,
                },
            )]),
            timeout: Duration::from_secs(10),
        },
    )
    .expect("build executor");
    let first = executor.execute(build_id).await.expect("isolated build");
    let duplicate = executor.execute(build_id).await.expect("idempotent replay");
    assert_eq!(first, duplicate);
    assert_eq!(first.artifact_count, 1);
    assert_eq!(provisions.load(Ordering::SeqCst), 1);
    let state: (String, String, i32, i32) = sqlx::query_as(
        "SELECT request.state, execution.state, execution.exit_code,
                jsonb_array_length(execution.logs)
         FROM build_requests AS request
         JOIN build_executions AS execution
           ON execution.build_request_id = request.id
         WHERE request.id = $1",
    )
    .bind(build_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("durable build result");
    assert_eq!(
        state,
        (String::from("succeeded"), String::from("drafted"), 0, 1)
    );
    let release_state: String = sqlx::query_scalar("SELECT state FROM releases WHERE id = $1")
        .bind(first.release_id.as_uuid())
        .fetch_one(&pool)
        .await
        .expect("draft release");
    assert_eq!(release_state, "draft");
    let (path, mode, storage_key): (String, i32, Uuid) = sqlx::query_as(
        "SELECT path, mode, storage_key
         FROM release_artifacts WHERE release_id = $1",
    )
    .bind(first.release_id.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("release artifact");
    assert_eq!((path.as_str(), mode), ("bin/agent", 0o555));
    assert_eq!(
        fs::metadata(artifact_root.join(storage_key.simple().to_string()))
            .expect("canonical object")
            .permissions()
            .mode()
            & 0o777,
        0o400
    );

    let imported_build = BuildRequestId::new();
    copy_build_request(&pool, build_id, imported_build, [7_u8; 32]).await;
    let recovered_output = root.join("recovered-output");
    fs::create_dir_all(recovered_output.join("bin")).expect("recovered output");
    let recovered_executable = recovered_output.join("bin/agent");
    fs::write(&recovered_executable, b"durably imported executable").expect("recovered executable");
    fs::set_permissions(&recovered_executable, fs::Permissions::from_mode(0o500))
        .expect("recovered executable mode");
    fs::set_permissions(&recovered_output, fs::Permissions::from_mode(0o500))
        .expect("sealed recovered output");
    let imported = artifact_store
        .import_for(imported_build.as_uuid(), &recovered_output)
        .expect("pre-crash artifact import");
    let release_id = ReleaseId::new();
    let release_agent_id = ReleaseAgentId::new();
    let artifact_id = ReleaseArtifactId::new();
    let artifact = imported.first().expect("one imported artifact");
    let manifest = serde_json::json!([{
        "id": artifact_id,
        "path": artifact.path,
        "kind": "executable",
        "mode": artifact.mode,
        "content_hash": artifact.content_hash,
        "size_bytes": artifact.size_bytes,
        "media_type": "application/x-hephaestus-test",
        "storage_key": artifact.storage_key,
    }]);
    sqlx::query("UPDATE build_requests SET state = 'importing' WHERE id = $1")
        .bind(imported_build.as_uuid())
        .execute(&pool)
        .await
        .expect("importing request");
    sqlx::query(
        "INSERT INTO build_executions
         (build_request_id, vm_id, release_id, release_agent_id,
          release_version, state, exit_code, artifact_manifest,
          sealed_at, imported_at)
         VALUES ($1, $2, $3, $4, $5, 'imported', 0, $6, now(), now())",
    )
    .bind(imported_build.as_uuid())
    .bind(format!("build-{imported_build}"))
    .bind(release_id.as_uuid())
    .bind(release_agent_id.as_uuid())
    .bind(format!(
        "build-{}",
        &imported_build.as_uuid().simple().to_string()[..16]
    ))
    .bind(manifest)
    .execute(&pool)
    .await
    .expect("durable imported boundary");
    let recovered = executor
        .execute(imported_build)
        .await
        .expect("resume imported build without VM");
    assert_eq!(recovered.release_id, release_id);
    assert_eq!(recovered.release_agent_id, release_agent_id);
    assert_eq!(provisions.load(Ordering::SeqCst), 1);
    let recovered_artifact: (Uuid, Uuid) =
        sqlx::query_as("SELECT id, storage_key FROM release_artifacts WHERE release_id = $1")
            .bind(release_id.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("recovered release artifact");
    assert_eq!(
        recovered_artifact,
        (artifact_id.as_uuid(), artifact.storage_key)
    );

    let failed_build = BuildRequestId::new();
    copy_build_request(&pool, build_id, failed_build, [6_u8; 32]).await;
    fail_next_provision.store(true, Ordering::SeqCst);
    assert!(matches!(
        executor.execute(failed_build).await,
        Err(BuildExecutionError::Vm)
    ));
    let failed_event: serde_json::Value = sqlx::query_scalar(
        "SELECT payload FROM outbox
         WHERE aggregate_type = 'release' AND aggregate_id = $1
           AND subject = 'hephaestus.build.failed.v1'",
    )
    .bind(failed_build.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("transactional build failure event");
    assert_eq!(failed_event["build_request_id"], failed_build.to_string());
    assert_eq!(failed_event["failure_code"], "vm_provision");
    assert_eq!(failed_event["schema_version"], 1);
    assert_eq!(failed_event["message_id"], failed_event["idempotency_key"]);
    assert!(failed_event["request_id"].is_null());
    assert!(failed_event["trace_id"].is_null());

    let denied_build = BuildRequestId::new();
    copy_build_request(&pool, build_id, denied_build, [8_u8; 32]).await;
    sqlx::query(
        "DELETE FROM project_maintainers
         WHERE project_id = (
             SELECT repository.project_id
             FROM build_requests request
             JOIN repositories repository ON repository.id = request.repository_id
             WHERE request.id = $1
         )",
    )
    .bind(denied_build.as_uuid())
    .execute(&pool)
    .await
    .expect("revoke build authority");
    assert!(matches!(
        executor.execute(denied_build).await,
        Err(BuildExecutionError::Unauthorized)
    ));
    assert_eq!(provisions.load(Ordering::SeqCst), 2);
    let denied_audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM authorization_audit_events
         WHERE object_type = 'build' AND object_id = $1
           AND permission = 'can_execute' AND decision = 'deny'",
    )
    .bind(denied_build.as_uuid())
    .fetch_one(&pool)
    .await
    .expect("denial audit");
    assert_eq!(denied_audits, 1);
}

async fn copy_build_request(
    pool: &sqlx::PgPool,
    source: BuildRequestId,
    destination: BuildRequestId,
    definition_hash: [u8; 32],
) {
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         SELECT $1, repository_id, source_commit, source_ref, origin_receive_id,
                $2, 'queued', created_by
         FROM build_requests WHERE id = $3",
    )
    .bind(destination.as_uuid())
    .bind(definition_hash.as_slice())
    .bind(source.as_uuid())
    .execute(pool)
    .await
    .expect("copy build request");
}

async fn seed(pool: &sqlx::PgPool, repository_id: Uuid, commit: &str) -> BuildRequestId {
    let user = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let receive = Uuid::new_v4();
    let build = BuildRequestId::new();
    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'Build Owner')")
        .bind(user)
        .execute(pool)
        .await
        .expect("user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, $2)")
        .bind(organization)
        .bind(format!("build-{organization}"))
        .execute(pool)
        .await
        .expect("organization");
    sqlx::query(
        "INSERT INTO organization_members (organization_id, user_id, role)
         VALUES ($1, $2, 'owner')",
    )
    .bind(organization)
    .bind(user)
    .execute(pool)
    .await
    .expect("owner");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, $3)")
        .bind(project)
        .bind(organization)
        .bind(format!("project-{project}"))
        .execute(pool)
        .await
        .expect("project");
    sqlx::query("INSERT INTO project_maintainers (project_id, user_id) VALUES ($1, $2)")
        .bind(project)
        .bind(user)
        .execute(pool)
        .await
        .expect("project maintainer");
    sqlx::query(
        "INSERT INTO repositories
         (id, project_id, name, default_branch, is_public)
         VALUES ($1, $2, $3, 'refs/heads/main', false)",
    )
    .bind(repository_id)
    .bind(project)
    .bind(format!("repository-{repository_id}"))
    .execute(pool)
    .await
    .expect("repository");
    sqlx::query(
        "INSERT INTO git_receives
         (id, repository_id, actor_id, principal, status, accepted_at)
         VALUES ($1, $2, $3, 'build-test', 'accepted', now())",
    )
    .bind(receive)
    .bind(repository_id)
    .bind(user)
    .execute(pool)
    .await
    .expect("receive");
    let parsed = agent_config::parse(CONFIG.as_bytes());
    let config = parsed.config.expect("valid build config");
    sqlx::query(
        "INSERT INTO agent_config_revisions
         (id, repository_id, receive_id, commit_sha, config_hash,
          normalized_config_hash, schema_version, status, config, diagnostics)
         VALUES ($1, $2, $3, $4, $5, $6, 2, 'valid', $7, '[]')",
    )
    .bind(Uuid::new_v4())
    .bind(repository_id)
    .bind(receive)
    .bind(commit)
    .bind(parsed.hash.as_str())
    .bind(parsed.normalized_hash.expect("normalized hash").as_str())
    .bind(serde_json::to_value(config).expect("serialize config"))
    .execute(pool)
    .await
    .expect("config revision");
    sqlx::query(
        "INSERT INTO build_requests
         (id, repository_id, source_commit, source_ref, origin_receive_id,
          build_definition_hash, state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, $5, 'queued', $6)",
    )
    .bind(build.as_uuid())
    .bind(repository_id)
    .bind(commit)
    .bind(receive)
    .bind([9_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("build request");
    build
}

fn source_repository(root: &Path, repository_root: &Path) -> (Uuid, String) {
    let source = root.join("source");
    fs::create_dir(&source).expect("source");
    git(&source, &["init", "--initial-branch=main"]);
    git(&source, &["config", "user.name", "Build Test"]);
    git(&source, &["config", "user.email", "build@example.invalid"]);
    fs::write(source.join("agent.toml"), CONFIG).expect("agent config");
    fs::write(source.join("input.txt"), "exact source\n").expect("source input");
    git(&source, &["add", "."]);
    git(&source, &["commit", "-m", "build source"]);
    let commit = git_text(&source, &["rev-parse", "HEAD"]);
    let repository_id = Uuid::new_v4();
    let bare = repository_root.join(format!("{repository_id}.git"));
    git(
        root,
        &[
            "clone",
            "--bare",
            source.to_str().expect("source path"),
            bare.to_str().expect("bare path"),
        ],
    );
    (repository_id, commit)
}

struct OutputProvider {
    provisions: Arc<AtomicUsize>,
    fail_next_provision: Arc<AtomicBool>,
}

#[async_trait]
impl VmProvider for OutputProvider {
    fn name(&self) -> &'static str {
        "isolated-build-test"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        self.provisions.fetch_add(1, Ordering::SeqCst);
        if self.fail_next_provision.swap(false, Ordering::SeqCst) {
            return Err(vm_io(std::io::Error::other(
                "intentional provision failure",
            )));
        }
        assert!(spec.disks.is_empty());
        assert_eq!(spec.command.env.len(), 0);
        let source = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-source")
            .expect("source mount");
        assert!(source.read_only);
        assert!(!source.host_path.join(".git").exists());
        let output = spec
            .mounts
            .iter()
            .find(|mount| mount.tag == "build-output")
            .expect("output mount");
        assert!(!output.read_only);
        Ok(Arc::new(OutputInstance::new(
            spec.id,
            output.host_path.clone(),
        )))
    }

    async fn cleanup_orphan(&self, _id: &vm_trait::VmId) -> Result<(), VmError> {
        Ok(())
    }
}

struct OutputInstance {
    id: vm_trait::VmId,
    output: PathBuf,
    events: broadcast::Sender<VmEvent>,
    exit: watch::Sender<Option<VmExit>>,
}

impl OutputInstance {
    fn new(id: vm_trait::VmId, output: PathBuf) -> Self {
        let (events, _) = broadcast::channel(16);
        let (exit, _) = watch::channel(None);
        Self {
            id,
            output,
            events,
            exit,
        }
    }
}

#[async_trait]
impl VmInstance for OutputInstance {
    fn id(&self) -> &vm_trait::VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        fs::create_dir(self.output.join("bin")).map_err(vm_io)?;
        let executable = self.output.join("bin/agent");
        fs::write(&executable, b"immutable built executable").map_err(vm_io)?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).map_err(vm_io)?;
        drop(self.events.send(VmEvent::Log {
            stream: LogStream::Stdout,
            bytes: b"build completed".to_vec(),
        }));
        drop(self.events.send(VmEvent::Metric(VmMetric {
            name: String::from("build.outputs"),
            value: 1.0,
            labels: BTreeMap::new(),
        })));
        let exit = VmExit {
            code: Some(0),
            signal: None,
        };
        drop(self.events.send(VmEvent::Exited(exit.clone())));
        self.exit.send_replace(Some(exit));
        Ok(())
    }

    async fn stop(&self, _mode: StopMode) -> Result<(), VmError> {
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut receiver = self.exit.subscribe();
        loop {
            let current = receiver.borrow().clone();
            if let Some(exit) = current {
                return Ok(exit);
            }
            receiver.changed().await.map_err(|_| VmError::Destroyed)?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        Ok(())
    }
}

fn vm_io(error: std::io::Error) -> VmError {
    VmError::Provider {
        provider: String::from("isolated-build-test"),
        code: String::from("fixture-io"),
        source: Box::new(error),
    }
}

fn git(directory: &Path, arguments: &[&str]) {
    let output = std::process::Command::new("/usr/bin/git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(
        output.status.success(),
        "Git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn git_text(directory: &Path, arguments: &[&str]) -> String {
    let output = std::process::Command::new("/usr/bin/git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .expect("run Git");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("Git UTF-8")
        .trim()
        .to_owned()
}
