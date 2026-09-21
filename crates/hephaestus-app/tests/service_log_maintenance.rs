#![cfg(feature = "test-fixtures")]

//! Application-level retention acceptance with the optional gateway edge off.

use hephaestus_app::{
    AppConfig, EXPECTED_DATABASE_MIGRATION, HephaestusApp, OidcConfig, RegistryConfig,
    RuntimePolicy, VmBackendConfig,
};
use jsonwebtoken::Algorithm;
use registry_domain::RegistryAuthority;
use registry_notification::CallbackCredential;
use registry_token::{RegistryTokenIssuer, SigningKey, TokenLifetime};
use secret_broker::DenyingBrokerAdapter;
use secret_runtime::EphemeralSecretConfig;
use secret_store::LocalKeyProvider;
use serial_test::serial;
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::{sleep, timeout};
use url::Url;
use uuid::Uuid;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

const TEST_SIGNING_SECRET: &[u8] = b"service-log-retention-test-signing-secret";
const TEST_ROOT_IMAGE: &str = "service-log-retention-test@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

struct IsolatedDatabase {
    database_name: String,
    maintenance_url: String,
    target_url: String,
    max_version: i64,
}

#[derive(sqlx::FromRow)]
struct RemainingEpoch {
    epoch_count: i64,
    min_fencing_token: Option<i64>,
    max_fencing_token: Option<i64>,
    min_acknowledged_through: Option<i64>,
    min_retained_bytes: Option<i64>,
    min_retained_chunks: Option<i64>,
}

impl IsolatedDatabase {
    async fn create(parent_url: &str) -> Self {
        let database_name = format!("hephaestus_service_log_{}", Uuid::new_v4().simple());
        let mut maintenance_url = Url::parse(parent_url).expect("parse PostgreSQL test URL");
        maintenance_url.set_path("/postgres");
        let mut target_url = Url::parse(parent_url).expect("parse PostgreSQL test URL");
        target_url.set_path(&format!("/{database_name}"));

        let maintenance_pool = PgPoolOptions::new()
            .max_connections(2)
            .connect(maintenance_url.as_str())
            .await
            .expect("connect PostgreSQL maintenance database");
        sqlx::query(&format!("CREATE DATABASE \"{database_name}\""))
            .execute(&maintenance_pool)
            .await
            .expect("create isolated PostgreSQL database");
        maintenance_pool.close().await;

        let target_pool = PgPoolOptions::new()
            .max_connections(5)
            .connect(target_url.as_str())
            .await
            .expect("connect isolated PostgreSQL database");
        sqlx::migrate!("../../migrations")
            .run(&target_pool)
            .await
            .expect("apply database migrations");
        let max_version: i64 = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT max(version) FROM _sqlx_migrations WHERE success",
        )
        .fetch_one(&target_pool)
        .await
        .expect("read isolated migration marker")
        .expect("isolated migrations exist");
        assert_eq!(max_version, EXPECTED_DATABASE_MIGRATION);
        target_pool.close().await;

        Self {
            database_name,
            maintenance_url: maintenance_url.to_string(),
            target_url: target_url.to_string(),
            max_version,
        }
    }

    async fn drop(self) {
        let maintenance_pool = PgPoolOptions::new()
            .max_connections(1)
            .connect(&self.maintenance_url)
            .await
            .expect("reconnect PostgreSQL maintenance database");
        sqlx::query(&format!("DROP DATABASE \"{}\"", self.database_name))
            .execute(&maintenance_pool)
            .await
            .expect("drop isolated PostgreSQL database");
        maintenance_pool.close().await;
    }
}

fn test_registry() -> RegistryConfig {
    RegistryConfig {
        token_issuer: Arc::new(RegistryTokenIssuer::new(
            "https://registry.invalid/token"
                .parse()
                .expect("registry issuer URL"),
            "registry.invalid"
                .parse()
                .expect("registry service authority"),
            SigningKey::hs256(
                "service-log-test".parse().expect("registry key id"),
                TEST_SIGNING_SECRET,
            )
            .expect("registry signing key"),
            TokenLifetime::new(300).expect("registry token lifetime"),
        )),
        notification_callback: CallbackCredential::parse(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )
        .expect("registry notification callback"),
        zot: registry_zot::ZotClientConfig::new(
            RegistryAuthority::parse("registry.invalid").expect("registry service"),
            "http://127.0.0.1:1/",
        )
        .expect("registry Zot configuration"),
        reconciliation_lease: Duration::from_secs(30),
        reconciliation_interval: Duration::from_secs(30),
    }
}

fn executable_git_http_backend() -> PathBuf {
    [
        PathBuf::from("/usr/lib/git-core/git-http-backend"),
        PathBuf::from("/usr/libexec/git-core/git-http-backend"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .expect("git-http-backend must be installed")
}

// Keep the disposable application fixture in one place so every path uses
// the same isolated roots and production configuration boundary.
#[allow(clippy::too_many_lines)]
fn app_config(database_url: String, nats_url: String, root: &Path) -> AppConfig {
    let repository_root = root.join("repositories");
    let volume_root = root.join("volumes");
    let workspace_root = root.join("workspaces");
    let artifact_root = root.join("artifacts");
    let runtime_root = root.join("runtime");
    let release_artifact_root = root.join("release-artifacts");
    let handoff_root = root.join("handoffs");
    let build_root = root.join("builds");
    let secret_mount_root = root.join("secret-mounts");
    let root_image = root.join("root-image");
    let broker_socket = root.join("secret-broker.sock");
    let hook = root.join("git-hooks/pre-receive");
    for path in [
        &repository_root,
        &volume_root,
        &workspace_root,
        &artifact_root,
        &runtime_root,
        &release_artifact_root,
        &handoff_root,
        &build_root,
        &secret_mount_root,
        &root_image,
    ] {
        fs::create_dir_all(path).expect("create application test root");
    }
    fs::set_permissions(&secret_mount_root, fs::Permissions::from_mode(0o700))
        .expect("set secret mount root mode");
    fs::create_dir_all(hook.parent().expect("hook parent")).expect("create hook root");
    fs::write(&hook, b"#!/bin/sh\nexit 1\n").expect("write fail-closed test hook");
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).expect("set test hook mode");

    let mut root_images = BTreeMap::new();
    root_images.insert(
        TEST_ROOT_IMAGE.to_owned(),
        RootFilesystem::Directory {
            host_path: root_image,
        },
    );
    AppConfig {
        database_url,
        nats_url,
        http_listen: "127.0.0.1:0".parse().expect("test HTTP address"),
        rpc_mediator_signing_key: hephaestus_app::rpc::mediator_signing_key(TEST_SIGNING_SECRET),
        repository_root: repository_root.clone(),
        git_http_backend: executable_git_http_backend(),
        git_pre_receive_hook: hook,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: String::from("https://issuer.service-log.invalid"),
            audience: String::from("service-log-test"),
            algorithm: Algorithm::HS256,
            decoding_key: jsonwebtoken::DecodingKey::from_secret(TEST_SIGNING_SECRET),
        },
        registry: test_registry(),
        gateway_edge: None,
        volumes: LocalVolumeConfig {
            volume_root,
            transient_runtime_roots: Vec::new(),
            host_id: String::from("service-log-test-host"),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: PathBuf::from("/usr/sbin/mkfs.ext4"),
        },
        workspaces: LocalWorkspaceConfig {
            workspace_root,
            artifact_root,
            repository_root,
            git_binary: PathBuf::from("/usr/bin/git"),
            limits: WorkspaceLimits::default(),
        },
        run_runtime: run_runtime_local::LocalRunRuntimeConfig {
            runtime_root,
            release_artifact_root,
        },
        runtime_authority_handoff_root: handoff_root,
        runtime_authority_handoff_key: [0x31; 32],
        runtime_authority_session_ttl: Duration::from_secs(3_600),
        build_workspace_root: build_root,
        build_timeout: Duration::from_secs(30),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: false,
        },
        secret_keys: LocalKeyProvider::new("service-log/v1", [("service-log/v1", [7_u8; 32])])
            .expect("test secret key"),
        secret_broker_socket: broker_socket,
        secret_broker_adapter: Arc::new(DenyingBrokerAdapter),
        vm_backend: VmBackendConfig::FixtureResult,
        root_images,
        oci_builder: None,
        runtime_policy: RuntimePolicy {
            version: String::from("service-log-test/v1"),
            max_vcpus: 2,
            max_memory_mib: 1_024,
            allow_broker_only: true,
            allow_egress: false,
        },
        agent_state_capacity_bytes: 16 * 1024 * 1024,
        worker_concurrency: 2,
        outbox_poll_interval: Duration::from_millis(50),
        outbox_batch_size: 20,
        startup_timeout: Duration::from_secs(20),
        shutdown_timeout: Duration::from_secs(20),
    }
}

// Keep the persisted gateway graph explicit so this acceptance fixture mirrors
// the production foreign-key and lifecycle shape without hidden helpers.
#[allow(clippy::too_many_lines)]
async fn seed_aged_cleaned_log(pool: &PgPool) -> Uuid {
    let user = Uuid::new_v4();
    let organization = Uuid::new_v4();
    let project = Uuid::new_v4();
    let repository = Uuid::new_v4();
    let gateway = Uuid::new_v4();
    let revision = Uuid::new_v4();
    let instance = Uuid::new_v4();
    let owner = Uuid::new_v4();
    let build_request = Uuid::new_v4();
    let release = Uuid::new_v4();
    let family = Uuid::new_v4();
    let release_agent = Uuid::new_v4();

    sqlx::query("INSERT INTO users (id, display_name) VALUES ($1, 'retention test user')")
        .bind(user)
        .execute(pool)
        .await
        .expect("seed retention user");
    sqlx::query("INSERT INTO organizations (id, name) VALUES ($1, 'retention test org')")
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed retention organization");
    sqlx::query("INSERT INTO projects (id, organization_id, name) VALUES ($1, $2, 'retention')")
        .bind(project)
        .bind(organization)
        .execute(pool)
        .await
        .expect("seed retention project");
    sqlx::query("INSERT INTO repositories (id, project_id, name) VALUES ($1, $2, 'retention')")
        .bind(repository)
        .bind(project)
        .execute(pool)
        .await
        .expect("seed retention repository");
    sqlx::query(
        "INSERT INTO build_requests
            (id, repository_id, source_commit, source_ref, build_definition_hash,
             state, created_by)
         VALUES ($1, $2, $3, 'refs/heads/main', $4, 'succeeded', $5)",
    )
    .bind(build_request)
    .bind(repository)
    .bind("0000000000000000000000000000000000000001")
    .bind([4_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention build request");
    sqlx::query(
        "INSERT INTO releases
            (id, repository_id, version, source_commit, source_ref, build_request_id,
             build_definition_hash, configuration, configuration_hash, manifest_hash,
             state, publication_actor_id, published_at)
         VALUES ($1, $2, '1.0.0', $3, 'refs/heads/main', $4, $5, '{}'::jsonb,
                 $6, $7, 'published', $8, now())",
    )
    .bind(release)
    .bind(repository)
    .bind("0000000000000000000000000000000000000001")
    .bind(build_request)
    .bind([4_u8; 32].as_slice())
    .bind([5_u8; 32].as_slice())
    .bind([6_u8; 32].as_slice())
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention release");
    sqlx::query(
        "INSERT INTO agent_families (id, repository_id, agent_key)
         VALUES ($1, $2, 'retention-agent')",
    )
    .bind(family)
    .bind(repository)
    .execute(pool)
    .await
    .expect("seed retention agent family");
    sqlx::query(
        "INSERT INTO release_agents
            (id, release_id, family_id, agent_key, display_name, runtime_contract,
             runtime_contract_hash, parameter_schema, secret_slot_schema, requires_state)
         VALUES ($1, $2, $3, 'retention-agent', 'Retention agent', '{}'::jsonb,
                 $4, '[]'::jsonb, '[]'::jsonb, false)",
    )
    .bind(release_agent)
    .bind(release)
    .bind(family)
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed retention release agent");
    sqlx::query(
        "INSERT INTO gateways
            (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'retention', 'enabled', $4)",
    )
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
            (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
             release_agent_key, handler_contract, exposure, parameters, service_log_capture_mode,
             normalized_hash, created_by, service_loopback_port, service_readiness_path,
             service_health_path)
         VALUES ($1, $2, $3, $4, $5, $6, 'retention-agent', 'http.service.v1',
                 'public', '{}'::jsonb, 'application', $7, $8, 18081, '/ready', '/health')",
    )
    .bind(revision)
    .bind(gateway)
    .bind(project)
    .bind(repository)
    .bind(release)
    .bind(release_agent)
    .bind(vec![0_u8; 32])
    .bind(user)
    .execute(pool)
    .await
    .expect("seed retention service revision");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway)
        .bind(revision)
        .execute(pool)
        .await
        .expect("activate retention revision");
    sqlx::query(
        "INSERT INTO gateway_service_instances
            (id, gateway_id, revision_id, owner_host_id, owner_uuid, fencing_token,
             vm_id, state, lease_expires_at, heartbeat_at, cleaned_at)
         VALUES ($1, $2, $3, 'retention-test-host', $4, 2,
                 $5, 'cleaned', clock_timestamp() + interval '1 hour',
                 clock_timestamp() - interval '2 days', clock_timestamp() - interval '2 days')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(owner)
    .bind(format!("gateway-service-{instance}"))
    .execute(pool)
    .await
    .expect("seed cleaned retention instance");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             updated_at)
         VALUES ($1, $2, $3, $4, 1, 0, 3, 1,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed aged retention epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_epochs
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             acknowledged_through, retained_bytes, retained_chunks,
             updated_at)
         VALUES ($1, $2, $3, $4, 2, 0, 0, 0,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .execute(pool)
    .await
    .expect("seed empty aged retention epoch");
    sqlx::query(
        "INSERT INTO gateway_service_log_chunks
            (instance_id, gateway_id, revision_id, project_id, fencing_token,
             sequence, stream, observed_at, bytes, stored_at)
         VALUES ($1, $2, $3, $4, 1, 0, 'stdout',
                 clock_timestamp() - interval '25 hours', $5,
                 clock_timestamp() - interval '25 hours')",
    )
    .bind(instance)
    .bind(gateway)
    .bind(revision)
    .bind(project)
    .bind(vec![1_u8, 2, 3])
    .execute(pool)
    .await
    .expect("seed aged retention payload");
    sqlx::query(
        "INSERT INTO gateway_service_log_project_usage
            (project_id, retained_bytes, retained_chunks, retained_epochs)
         VALUES ($1, 3, 1, 2)",
    )
    .bind(project)
    .execute(pool)
    .await
    .expect("seed consistent retention usage");
    project
}

async fn wait_for_retention(pool: &PgPool, project: Uuid) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let row = timeout(
            Duration::from_secs(2),
            sqlx::query(
            "SELECT
                (SELECT count(*) FROM gateway_service_log_chunks WHERE project_id = $1) AS chunks,
                (SELECT count(*) FROM gateway_service_log_epochs WHERE project_id = $1) AS epochs,
                (SELECT retained_bytes FROM gateway_service_log_project_usage WHERE project_id = $1) AS bytes,
                (SELECT retained_chunks FROM gateway_service_log_project_usage WHERE project_id = $1) AS usage_chunks,
                (SELECT retained_epochs FROM gateway_service_log_project_usage WHERE project_id = $1) AS usage_epochs,
                (SELECT count(*)
                   FROM gateway_service_instances instance
                   JOIN gateways gateway ON gateway.id = instance.gateway_id
                  WHERE gateway.project_id = $1 AND instance.state <> 'cleaned') AS live_instances",
            )
            .bind(project)
            .fetch_one(pool),
        )
        .await
        .expect("bounded automatic retention query")
        .expect("query automatic retention result");
        let chunks: i64 = row.get("chunks");
        let epochs: i64 = row.get("epochs");
        let bytes: i64 = row.get("bytes");
        let usage_chunks: i64 = row.get("usage_chunks");
        let usage_epochs: i32 = row.get("usage_epochs");
        let live_instances: i64 = row.get("live_instances");
        if chunks == 0
            && epochs == 1
            && bytes == 0
            && usage_chunks == 0
            && usage_epochs == 1
            && live_instances == 0
        {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "automatic service-log retention did not complete: chunks={chunks} epochs={epochs} bytes={bytes} usage_chunks={usage_chunks} usage_epochs={usage_epochs} live_instances={live_instances}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

async fn cleanup_nats(nats_url: &str) {
    let client = async_nats::connect(nats_url)
        .await
        .expect("connect NATS cleanup client");
    let context = async_nats::jetstream::new(client);
    for stream in [
        "HEPH_RUN_COMMANDS",
        "HEPH_RUN_EVENTS",
        "HEPHAESTUS_GIT_EVENTS",
        "HEPHAESTUS_RELEASE_EVENTS",
        "HEPHAESTUS_PRODUCT_EVENTS",
    ] {
        if let Err(error) = context.delete_stream(stream).await {
            assert!(
                error.to_string().contains("stream not found"),
                "delete test NATS stream: {error}"
            );
        }
    }
}

fn require_disposable_nats(nats_url: &str) {
    let parsed = Url::parse(nats_url).expect("parse NATS test URL");
    assert_eq!(parsed.scheme(), "nats", "retention test requires NATS URL");
    assert!(
        matches!(parsed.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
        "retention test refuses non-loopback NATS URL"
    );
}

const fn retention_supports_migration(version: i64) -> bool {
    version >= 81
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn application_retention_runs_without_gateway_edge_and_closes_worker_pool() {
    if std::env::var("REAL_APP_SERVICE_LOG_MAINTENANCE").as_deref() != Ok("1") {
        eprintln!(
            "SKIPPED application service-log retention: set REAL_APP_SERVICE_LOG_MAINTENANCE=1"
        );
        return;
    }
    assert!(
        retention_supports_migration(EXPECTED_DATABASE_MIGRATION),
        "retention requires migration 81 or newer"
    );
    let parent_database_url = std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
        .expect("HEPHAESTUS_POSTGRES_TEST_URL is required for real retention validation");
    let nats_url = std::env::var("HEPHAESTUS_NATS_TEST_URL")
        .expect("HEPHAESTUS_NATS_TEST_URL is required for real retention validation");
    require_disposable_nats(&nats_url);
    let isolated = IsolatedDatabase::create(&parent_database_url).await;
    let root = tempfile::tempdir().expect("application retention test root");
    let seed_pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&isolated.target_url)
        .await
        .expect("connect isolated seed pool");
    let project = seed_aged_cleaned_log(&seed_pool).await;
    seed_pool.close().await;

    let app = HephaestusApp::build(app_config(
        isolated.target_url.clone(),
        nats_url.clone(),
        root.path(),
    ))
    .await
    .expect("build application with gateway edge disabled")
    .start()
    .await
    .expect("start application with gateway edge disabled");
    let service_log_pool = app.service_log_pool_for_test();
    wait_for_retention(&service_log_pool, project).await;
    let remaining_epoch: RemainingEpoch = sqlx::query_as(
        "SELECT count(*) AS epoch_count,
                min(fencing_token) AS min_fencing_token,
                max(fencing_token) AS max_fencing_token,
                min(acknowledged_through) AS min_acknowledged_through,
                min(retained_bytes) AS min_retained_bytes,
                min(retained_chunks) AS min_retained_chunks
           FROM gateway_service_log_epochs WHERE project_id = $1",
    )
    .bind(project)
    .fetch_one(&service_log_pool)
    .await
    .expect("read post-retention epoch metadata");
    assert_eq!(
        (
            remaining_epoch.epoch_count,
            remaining_epoch.min_fencing_token,
            remaining_epoch.max_fencing_token,
            remaining_epoch.min_acknowledged_through,
            remaining_epoch.min_retained_bytes,
            remaining_epoch.min_retained_chunks,
        ),
        (1, Some(1), Some(1), Some(0), Some(0), Some(0)),
        "empty aged epoch should be GC'd while the payload epoch remains as fresh metadata"
    );
    app.shutdown()
        .await
        .expect("shutdown application after retention");
    assert!(
        service_log_pool.is_closed(),
        "dedicated service-log pool closes"
    );

    cleanup_nats(&nats_url).await;
    let max_version = isolated.max_version;
    isolated.drop().await;
    println!("REAL_APP_SERVICE_LOG_MAINTENANCE=1 max_migration={max_version}");
}
