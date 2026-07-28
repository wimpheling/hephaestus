//! Opt-in daemon-level golden path through every production boundary.

use async_trait::async_trait;
use forge_domain::{GitRef, OrganizationId};
use forge_service::{CreateRepository, GitStorage, PgForgeRepository};
use hephaestus_app::{AppConfig, HephaestusApp, OidcConfig, RunEventKind, VmBackendConfig};
use identity_domain::UserId;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serial_test::serial;
use sqlx::postgres::PgPoolOptions;
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
const ROOT_IMAGE: &str = "golden-root@sha256:provider-neutral";

#[tokio::test(flavor = "multi_thread")]
#[serial]
#[allow(clippy::too_many_lines)]
async fn bearer_push_starts_run_through_production_bootstrap() {
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

    let backend_fixture = backend_fixture(&root).await;
    let mut transient_runtime_roots = backend_fixture.transient_runtime_roots;
    transient_runtime_roots.push(root.join("workspaces"));
    let backend = git_backend().await;
    let app = HephaestusApp::build(AppConfig {
        database_url,
        nats_url: nats_url.clone(),
        http_listen: "127.0.0.1:0".parse().expect("ephemeral listen address"),
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
        vm_backend: backend_fixture.backend,
        root_images: BTreeMap::from([(
            String::from(ROOT_IMAGE),
            RootFilesystem::Directory {
                host_path: backend_fixture.root_image,
            },
        )]),
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
    running
        .wait_for_run_event(
            run_id,
            RunEventKind::ResultCompleted,
            Duration::from_secs(10),
        )
        .await
        .expect("persisted result completion");

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
         WHERE actor_id = $1 AND object_id IN ($2, (
             SELECT agent_id FROM run_requests WHERE run_id = $3
         ))
         ORDER BY permission",
    )
    .bind(user_id.as_uuid())
    .bind(repository.id.as_uuid())
    .bind(run_id.as_uuid())
    .fetch_all(&pool)
    .await
    .expect("authorization audit");
    assert!(permissions.iter().any(|value| value == "can_write"));
    assert!(permissions.iter().any(|value| value == "can_execute"));
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
version = 1
[agent]
name = "golden-agent"
[guest]
command = "/bin/sh"
arguments = ["-c", "printf 'agent edit\n' > input.txt; printf 'durable report\n' > reports/result.txt"]
working_directory = "/workspace/work"
[resources]
vcpus = 1
memory_mib = 128
[root_image]
reference = "golden-root@sha256:provider-neutral"
[workspace]
mount = true
path = "/workspace/repo"
read_only = true
[state_volume]
enabled = true
[results]
declared_files = ["reports/result.txt"]
[network]
profile = "disabled"
[triggers]
push = true
refs = ["refs/heads/main"]
"#
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
    ] {
        drop(context.delete_stream(stream).await);
    }
}
