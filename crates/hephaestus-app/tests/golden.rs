//! Opt-in daemon-level golden path through every production boundary.

use authz_postgres::PostgresMelangeAuthorizer;
use brokered_egress_domain::{
    BrokeredSecretRule, BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
};
use forge_domain::{GitRef, OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::{CreateRepository, GitStorage};
use hephaestus_app::{
    AppConfig, GatewayEdgeConfig, HephaestusApp, OidcConfig, RegistryConfig, RunEventKind,
};
use identity_domain::{AuthenticatedIdentity, RequestId, UserId};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use mailbox_domain::{
    BodyReference, BodyReferenceId, ContentMetadata, DeduplicationKey, EnvelopeMethod,
    EnvelopeRoute, MailboxEnvelope, MailboxEvent, MailboxId, ProducerId,
};
use mailbox_postgres::PostgresMailboxRepository;
use run_runtime_local::LocalRunRuntimeConfig;
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_broker::{BrokeredHttpsAdapterRegistry, DenyingBrokerAdapter};
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretCommandKey,
    SecretGrantId, SecretId, SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget,
    SecretUsePolicy, SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_runtime::EphemeralSecretConfig;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serial_test::serial;
use sha2::{Digest, Sha256};
use sqlx::{Row, postgres::PgPoolOptions};
use std::{
    collections::BTreeMap,
    env,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use time::OffsetDateTime;
use tokio::process::Command;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;
use vm_trait::RootFilesystem;
use volume_local::LocalVolumeConfig;
use workspace_local::{LocalWorkspaceConfig, WorkspaceLimits};

#[path = "../../../examples/cooking/tests/scenario.rs"]
mod cooking;
#[path = "../../../examples/cooking/tests/inspection.rs"]
mod cooking_inspection;
mod support;

use support::backend_fixture;

const ISSUER: &str = "https://issuer.golden.invalid";
const AUDIENCE: &str = "hephaestus-git";
const SIGNING_SECRET: &[u8] = b"golden-test-signing-secret-with-sufficient-entropy";
const ROOT_IMAGE: &str =
    "golden-root@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const BROKERED_E2E_RULE_ID: uuid::Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000002");
const BROKERED_E2E_SENTINEL: &str = "golden-brokered-provider-sentinel-5d1a";
const GATEWAY_HANDLER: &str =
    "#!/bin/sh\nexec /usr/libexec/hephaestus/integration-check --private-http-brokered-mailbox\n";
type MailboxTimeoutEvidence = (
    i32,
    uuid::Uuid,
    String,
    String,
    String,
    Option<String>,
    Option<String>,
);
const GOLDEN_AGENT: &str = r#"#!/bin/sh
  set -eu
  if test -r /run/hephaestus/mailbox-event.json; then
      if grep -q '"route":"/gateway/golden-proof"' /run/hephaestus/mailbox-event.json; then
          test "$(cat /run/hephaestus/mailbox-body)" = "gateway-real-mailbox-body"
      else
          grep -q '"route":"/mailbox/golden-proof"' /run/hephaestus/mailbox-event.json
          test "$(cat /run/hephaestus/mailbox-body)" = "golden-real-mailbox-body"
      fi
      printf 'mailbox-body-ok\n' > /var/lib/hephaestus/golden-state
      exit 0
  fi
  if test -x /usr/libexec/hephaestus/integration-check \
      && test -r /run/hephaestus-secrets/.runtime-credential; then
      exec /usr/libexec/hephaestus/integration-check --brokered-https-e2e
  fi
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
    let libkrun_e2e = env::var("HEPHAESTUS_APP_LIBKRUN_E2E").as_deref() == Ok("1");
    let gateway_caddy_e2e = env::var("HEPHAESTUS_APP_GATEWAY_CADDY_E2E").as_deref() == Ok("1");
    assert!(
        !cooking::enabled() || gateway_caddy_e2e,
        "cooking requires the joined Caddy/libkrun fixture"
    );
    assert!(
        !gateway_caddy_e2e || libkrun_e2e,
        "the joined Caddy gateway proof requires the real libkrun backend"
    );
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
    // The config is resolved by the production Git-receive path, so seed the
    // same immutable catalog record that the runtime maps to the libkrun root
    // filesystem below. This keeps the composed proof on the real resolver
    // rather than retaining the previous, now-invalid ad-hoc image syntax.
    sqlx::query(
        "INSERT INTO oci_images
             (id, key, display_name, image_reference, toolchains, architectures,
              availability_state, provenance, platform_policy_version)
         VALUES ($1, 'golden-root', 'Golden root', $2, '[]'::jsonb,
                 ARRAY['x86_64'], 'available', '{}'::jsonb, 'golden/v1')",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(ROOT_IMAGE)
    .execute(&pool)
    .await
    .expect("seed selected OCI image");
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
    let seeded_instance = seed_reusable_instance(
        &pool,
        user_id,
        project.id.as_uuid(),
        repository.id.as_uuid(),
        &root.join("release-artifacts"),
        libkrun_e2e,
    )
    .await;
    let mut brokered_fixture = if libkrun_e2e {
        Some(if cooking::enabled() {
            cooking::seed_brokered_fixture(
                &pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        } else {
            seed_brokered_https_fixture(
                &pool,
                user_id,
                organization_id,
                project.id.as_uuid(),
                &seeded_instance,
            )
            .await
        })
    } else {
        None
    };
    // This mailbox is created before the daemon starts so the released
    // gateway revision can be bound to it immutably.  The guest only sees the
    // symbolic `deliver` slot, never this UUID or the producer identity.
    let gateway_mailbox = if gateway_caddy_e2e {
        let mailbox_id = MailboxId::new();
        PostgresMailboxRepository::new(pool.clone())
            .ensure_mailbox(
                project.id.as_uuid(),
                mailbox_id,
                runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
            )
            .await
            .expect("create bound gateway golden mailbox");
        Some(mailbox_id)
    } else {
        None
    };
    let gateway_edge = if gateway_caddy_e2e {
        let gateway_agent =
            seed_gateway_release_agent(&pool, &seeded_instance, &root.join("release-artifacts"))
                .await;
        let fixture = brokered_fixture
            .as_ref()
            .expect("joined gateway proof has brokered fixture authority");
        let gateway_fixture = seed_gateway_brokered_route(
            &pool,
            user_id,
            project.id.as_uuid(),
            repository.id.as_uuid(),
            seeded_instance.release,
            gateway_agent,
            fixture.import_id,
            fixture.version_id,
            gateway_mailbox.expect("joined gateway mailbox"),
        )
        .await;
        let dispatcher = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("reserve gateway dispatcher listener");
        let dispatcher_listen = dispatcher
            .local_addr()
            .expect("gateway dispatcher listener address");
        drop(dispatcher);
        Some((
            GatewayEdgeConfig {
                caddy_admin_url: env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL")
                    .expect("joined Caddy admin URL"),
                caddy_configuration_template: caddy_configuration(
                    &env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL"),
                ),
                caddy_server_name: String::from("shared"),
                dispatcher_listen,
                public_authority: String::from("gateway.golden.invalid"),
            },
            gateway_fixture,
        ))
    } else {
        None
    };

    let backend_fixture = backend_fixture(&root).await;
    let mut transient_runtime_roots = backend_fixture.transient_runtime_roots;
    transient_runtime_roots.push(root.join("workspaces"));
    let backend = git_backend().await;
    let git_pre_receive_hook = root.join("git-hooks/pre-receive");
    std::fs::create_dir_all(
        git_pre_receive_hook
            .parent()
            .expect("Git hook has a parent directory"),
    )
    .expect("Git hook directory");
    std::fs::write(&git_pre_receive_hook, b"#!/bin/sh\nexit 1\n")
        .expect("write fail-closed Git hook fixture");
    std::fs::set_permissions(
        &git_pre_receive_hook,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("Git hook fixture mode");
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
        git_pre_receive_hook,
        git_http_limits: git_http::GitHttpLimits::default(),
        oidc: OidcConfig {
            issuer: String::from(ISSUER),
            audience: String::from(AUDIENCE),
            algorithm: Algorithm::HS256,
            decoding_key: jsonwebtoken::DecodingKey::from_secret(SIGNING_SECRET),
        },
        registry: RegistryConfig {
            token_issuer: Arc::new(registry_token::RegistryTokenIssuer::new(
                "https://forge.golden.invalid/v1/registry/token"
                    .parse()
                    .expect("registry issuer"),
                "registry.golden.invalid".parse().expect("registry service"),
                registry_token::SigningKey::hs256(
                    "golden-v1".parse().expect("registry key id"),
                    SIGNING_SECRET,
                )
                .expect("registry signing key"),
                registry_token::TokenLifetime::new(300).expect("registry token lifetime"),
            )),
            notification_callback: registry_notification::CallbackCredential::parse(
                "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
            )
            .expect("registry notification callback"),
            zot: registry_zot::ZotClientConfig::new(
                registry_domain::RegistryAuthority::parse("registry.golden.invalid")
                    .expect("registry authority"),
                "http://127.0.0.1:1/",
            )
            .expect("Zot client configuration"),
            reconciliation_lease: Duration::from_secs(30),
            reconciliation_interval: Duration::from_secs(30),
        },
        gateway_edge: gateway_edge.as_ref().map(|(config, _)| config.clone()),
        volumes: LocalVolumeConfig {
            volume_root: backend_fixture.volume_root,
            transient_runtime_roots,
            host_id: String::from("golden-host"),
            lease_duration: Duration::from_secs(30),
            mkfs_ext4: mkfs_ext4(),
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
        runtime_authority_handoff_root: root.join("runtime-authority-handoffs"),
        runtime_authority_handoff_key: [0x39; 32],
        runtime_authority_session_ttl: Duration::from_secs(3_600),
        build_workspace_root: root.join("isolated-builds"),
        build_timeout: Duration::from_secs(30),
        secret_mounts: EphemeralSecretConfig {
            root: secret_mount_root,
            require_memory_filesystem: false,
        },
        secret_keys: LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
            .expect("secret key"),
        secret_broker_socket: root.join("secret-broker.sock"),
        secret_broker_adapter: brokered_fixture.as_ref().map_or_else(
            || Arc::new(DenyingBrokerAdapter) as Arc<dyn secret_application::BrokerAdapter>,
            |fixture| fixture.upstream.adapter(),
        ),
        vm_backend: backend_fixture.backend,
        root_images: BTreeMap::from([(
            String::from(ROOT_IMAGE),
            RootFilesystem::Directory {
                host_path: backend_fixture.root_image,
            },
        )]),
        oci_builder: None,
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
    if cooking::enabled() {
        cooking::copy_blog(&source).await;
    }
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
    // Keep the fixture closed through startup recovery, then admit the exact
    // authenticated push that this golden proof is about.
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(seeded_instance.instance)
        .execute(&pool)
        .await
        .expect("open golden push run gate");
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
            Duration::from_secs(if libkrun_e2e { 60 } else { 10 }),
        )
        .await;
    if result_wait.is_err() {
        diagnose_golden_timeout(&pool, repository.id.as_uuid(), run_id).await;
    }
    result_wait.expect("persisted result completion");

    if cooking::enabled() {
        cooking::exercise(
            &pool,
            &running,
            &seeded_instance,
            &gateway_edge.as_ref().expect("cooking gateway").1,
            &root,
            repository.id.as_uuid(),
            &input_commit,
        )
        .await;
        brokered_fixture
            .take()
            .expect("cooking broker")
            .upstream
            .assert_substituted_request()
            .await;
        running.shutdown().await.expect("cooking daemon shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

    if libkrun_e2e {
        if gateway_caddy_e2e {
            let admin_url =
                env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL").expect("joined Caddy admin URL");
            let applied_config = reqwest::Client::new()
                .get(format!("{admin_url}/config/"))
                .send()
                .await
                .expect("load applied Caddy configuration")
                .error_for_status()
                .expect("Caddy configuration request succeeds")
                .text()
                .await
                .expect("read applied Caddy configuration");
            assert!(
                applied_config.contains("/gateway/brokered"),
                "Caddy must contain the authoritative gateway route before the public proof: {applied_config}"
            );
            let public_url =
                env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL").expect("joined Caddy public URL");
            let client = reqwest::Client::new();
            let first = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-a")
                .send();
            let second = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-b")
                .send();
            let (response, distinct) = tokio::join!(first, second);
            let response = response.expect("first concurrent Caddy gateway request");
            let distinct = distinct.expect("second concurrent Caddy gateway request");
            if response.status() != reqwest::StatusCode::CREATED
                || distinct.status() != reqwest::StatusCode::CREATED
            {
                let gateway_diagnostics: (i64, i64, i64, i64, i64, Vec<String>, Vec<String>) =
                    sqlx::query_as(
                        "SELECT
                         (SELECT count(*) FROM gateway_invocations),
                         (SELECT count(*) FROM gateway_authorization_snapshots),
                         (SELECT count(*) FROM gateway_authorization_snapshot_bindings),
                         (SELECT count(*) FROM gateway_runtime_authority_sessions),
                         (SELECT count(*) FROM gateway_mailbox_publications),
                         COALESCE((SELECT array_agg(status ORDER BY id)
                                   FROM gateway_runtime_authority_sessions), ARRAY[]::text[]),
                         COALESCE((SELECT array_agg(outcome ORDER BY id)
                                   FROM gateway_invocations), ARRAY[]::text[])",
                    )
                    .fetch_one(&pool)
                    .await
                    .expect("load gateway failure diagnostics");
                panic!(
                    "joined gateway requests failed: first={}, second={}, invocations={}, snapshots={}, snapshot_bindings={}, sessions={}, publications={}, session_statuses={:?}, invocation_outcomes={:?}",
                    response.status(),
                    distinct.status(),
                    gateway_diagnostics.0,
                    gateway_diagnostics.1,
                    gateway_diagnostics.2,
                    gateway_diagnostics.3,
                    gateway_diagnostics.4,
                    gateway_diagnostics.5,
                    gateway_diagnostics.6,
                );
            }
            assert_eq!(response.status(), reqwest::StatusCode::CREATED);
            assert_eq!(distinct.status(), reqwest::StatusCode::CREATED);
            assert_eq!(
                response.bytes().await.expect("gateway response body"),
                "gateway-brokered-header-ok"
            );
            assert_eq!(
                distinct
                    .bytes()
                    .await
                    .expect("distinct gateway response body"),
                "gateway-brokered-header-ok"
            );
            // A handler retry emits the same application-supplied key. It is
            // a fresh gateway invocation but must retain the first logical
            // mailbox event rather than delivering a second one.
            let retry = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-a")
                .send()
                .await
                .expect("Caddy gateway retry");
            assert_eq!(retry.status(), reqwest::StatusCode::CREATED);
            assert_eq!(
                retry.bytes().await.expect("gateway retry response body"),
                "gateway-brokered-header-ok"
            );
            let evidence: (String, bool, bool) = sqlx::query_as(
                "SELECT invocation.outcome, session.acknowledged_at IS NOT NULL,
                        EXISTS (
                            SELECT 1 FROM gateway_secret_leases AS lease
                             WHERE lease.invocation_id = invocation.id
                        )
                   FROM gateway_invocations AS invocation
                   JOIN gateway_runtime_authority_sessions AS session
                     ON session.invocation_id = invocation.id
                  WHERE invocation.request_id IS NOT NULL
                  ORDER BY invocation.accepted_at DESC LIMIT 1",
            )
            .fetch_one(&pool)
            .await
            .expect("persisted joined gateway authority evidence");
            assert_eq!(evidence.0, "completed");
            assert!(evidence.1, "gateway VM acknowledged its runtime authority");
            assert!(
                evidence.2,
                "gateway invocation held its inbound secret lease"
            );
            let mailbox = gateway_edge
                .as_ref()
                .expect("joined gateway fixture")
                .1
                .mailbox_id;
            let (accepted_publications, duplicate_publications): (i64, i64) = sqlx::query_as(
                "SELECT count(*) FILTER (WHERE publication.outcome = 'accepted'),
                        count(*) FILTER (WHERE publication.outcome = 'duplicate')
                   FROM gateway_mailbox_publications AS publication
                   JOIN gateway_invocations AS invocation
                     ON invocation.id = publication.invocation_id
                  WHERE publication.mailbox_id = $1",
            )
            .bind(mailbox.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("gateway mailbox publication provenance");
            assert_eq!((accepted_publications, duplicate_publications), (2, 1));
            let (event_count, wake_count, leaked_body): (i64, i64, bool) = sqlx::query_as(
                "SELECT
                     (SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1),
                     (SELECT count(*) FROM outbox
                       WHERE subject = 'heph.mailbox.v1.wake'
                         AND id IN (SELECT id FROM mailbox_events WHERE mailbox_id = $1)),
                     EXISTS (
                       SELECT 1 FROM gateway_mailbox_publications
                        WHERE mailbox_id = $1
                          AND to_jsonb(gateway_mailbox_publications)::text
                              LIKE '%gateway-real-mailbox-body%'
                     )",
            )
            .bind(mailbox.as_uuid())
            .fetch_one(&pool)
            .await
            .expect("gateway mailbox acceptance evidence");
            assert_eq!((event_count, wake_count), (2, 2));
            assert!(!leaked_body, "gateway publication provenance is value-free");

            // This waits for the event emitted by the real libkrun guest to
            // cross the transactional outbox and JetStream dispatcher into a
            // separate real target-agent run. The target command rejects an
            // altered route or body before it can succeed.
            let delivery = tokio::time::timeout(Duration::from_secs(60), async {
                loop {
                    let row = sqlx::query_as::<_, (i64, i64)>(
                        "SELECT count(DISTINCT delivery.event_id)
                                  FILTER (WHERE delivery.disposition = 'delivered'),
                                count(*) FILTER (
                                    WHERE run.state = 'cleaned_up'
                                      AND run.outcome = 'succeeded'
                                )
                           FROM mailbox_deliveries AS delivery
                           JOIN mailbox_delivery_attempts AS attempt
                             ON attempt.event_id = delivery.event_id
                           JOIN runs AS run ON run.id = attempt.run_id
                          WHERE delivery.mailbox_id = $1",
                    )
                    .bind(mailbox.as_uuid())
                    .fetch_one(&pool)
                    .await
                    .expect("gateway mailbox delivery evidence");
                    if row == (2, 2) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(25)).await;
                }
            })
            .await;
            if delivery.is_err() {
                let failures: Vec<(String, String, Option<String>, Option<String>)> =
                    sqlx::query_as(
                        "SELECT delivery.disposition, run.state, run.outcome, run.failure
                     FROM mailbox_delivery_attempts AS attempt
                     JOIN mailbox_deliveries AS delivery ON delivery.event_id = attempt.event_id
                     JOIN runs AS run ON run.id = attempt.run_id
                     WHERE delivery.mailbox_id = $1 ORDER BY run.id",
                    )
                    .bind(mailbox.as_uuid())
                    .fetch_all(&pool)
                    .await
                    .expect("gateway delivery failures");
                panic!("real gateway publications reach target-agent dispatch: {failures:?}");
            }

            // New invocations are denied after the exact bound grant is
            // revoked. This runs after the already accepted events settled so
            // it proves live authorization without perturbing their delivery.
            let fixture = &gateway_edge.as_ref().expect("joined gateway fixture").1;
            sqlx::query(
                "UPDATE gateway_mailbox_binding_grants
                    SET status = 'revoked', revoked_at = now(), revoked_by = $2
                  WHERE id = $1",
            )
            .bind(fixture.grant_id)
            .bind(user_id.as_uuid())
            .execute(&pool)
            .await
            .expect("revoke bound gateway mailbox grant");
            let denied = client
                .post(format!("{public_url}/gateway/brokered?mode=real"))
                .header("x-webhook-secret", BROKERED_E2E_SENTINEL)
                .body("gateway-brokered-request-denied")
                .send()
                .await
                .expect("revoked Caddy gateway request");
            assert_eq!(denied.status(), reqwest::StatusCode::BAD_GATEWAY);
            let accepted_after_revoke: i64 =
                sqlx::query_scalar("SELECT count(*) FROM mailbox_events WHERE mailbox_id = $1")
                    .bind(mailbox.as_uuid())
                    .fetch_one(&pool)
                    .await
                    .expect("revoked grant does not add mailbox events");
            assert_eq!(accepted_after_revoke, 2);
        }
        if let Some(fixture) = brokered_fixture.take() {
            fixture.upstream.assert_substituted_request().await;
        }
        running.shutdown().await.expect("graceful daemon shutdown");
        cleanup_streams(&nats_url).await;
        return;
    }

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

    // A durable mailbox journey reuses the production daemon, PostgreSQL
    // outbox, JetStream worker, run gate, fenced state volume, runtime
    // authority and configured VM backend. With `HEPHAESTUS_APP_LIBKRUN_E2E`
    // this is an actual libkrun guest, not a provider double.
    // Keep the fixture behind its gate during daemon startup. Startup/update
    // reconciliation must not acquire this state volume before this test's
    // mailbox command reaches the dispatch-time gate recheck.
    sqlx::query("UPDATE agent_instances SET run_gate_open = true WHERE id = $1")
        .bind(seeded_instance.instance)
        .execute(&pool)
        .await
        .expect("open mailbox proof run gate");
    let mailbox_repository = PostgresMailboxRepository::new(pool.clone());
    let mailbox_id = MailboxId::new();
    mailbox_repository
        .ensure_mailbox(
            project.id.as_uuid(),
            mailbox_id,
            runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        )
        .await
        .expect("create durable golden mailbox");
    let mailbox_body = b"golden-real-mailbox-body";
    let mailbox_event = MailboxEvent {
        id: mailbox_domain::MailboxEventId::new(),
        mailbox_id,
        instance_id: runtime_types::AgentInstanceId::from_uuid(seeded_instance.instance),
        producer_id: ProducerId::parse("golden-daemon-e2e").expect("producer"),
        deduplication_key: DeduplicationKey::parse("golden-mailbox-1").expect("deduplication"),
        envelope: MailboxEnvelope::new(
            EnvelopeMethod::parse("POST").expect("method"),
            EnvelopeRoute::parse("/mailbox/golden-proof").expect("route"),
            BTreeMap::new(),
            ContentMetadata::new(
                BodyReference::new(
                    BodyReferenceId::new(),
                    u32::try_from(mailbox_body.len()).expect("mailbox body length"),
                    Sha256::digest(mailbox_body).into(),
                )
                .expect("body reference"),
                Some(String::from("application/octet-stream")),
                Some(String::from("identity")),
            )
            .expect("content metadata"),
            OffsetDateTime::now_utc(),
            None,
        )
        .expect("mailbox envelope"),
    };
    let accepted = mailbox_repository
        .accept(
            project.id.as_uuid(),
            &mailbox_event,
            mailbox_body,
            u32::try_from(mailbox_body.len()).expect("mailbox body length"),
        )
        .await
        .expect("accept mailbox event through PostgreSQL");
    let mailbox_completion = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            let row = sqlx::query_as::<
                _,
                (
                    String,
                    uuid::Uuid,
                    String,
                    Option<String>,
                    Option<uuid::Uuid>,
                    Option<uuid::Uuid>,
                    Option<i64>,
                ),
            >(
                "SELECT delivery.disposition, attempt.run_id, run.state, run.outcome,
                        attempt.authorization_snapshot_id, attempt.lease_id,
                        attempt.lease_fencing_token
                 FROM mailbox_deliveries AS delivery
                 JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                 JOIN runs AS run ON run.id = attempt.run_id
                 WHERE delivery.event_id = $1
                 ORDER BY attempt.attempt_number DESC LIMIT 1",
            )
            .bind(accepted.event_id.as_uuid())
            .fetch_optional(&pool)
            .await
            .expect("load composed mailbox delivery");
            if let Some((disposition, run_id, state, outcome, snapshot, lease, fence)) = row {
                if disposition == "delivered" {
                    assert_eq!(state, "cleaned_up");
                    assert_eq!(outcome.as_deref(), Some("succeeded"));
                    assert!(snapshot.is_some(), "runtime authority snapshot is durable");
                    assert!(
                        lease.is_some() && fence.is_some(),
                        "fenced state lease is durable"
                    );
                    break run_id;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await;
    if mailbox_completion.is_err() {
        let evidence: Vec<MailboxTimeoutEvidence> = sqlx::query_as(
            "SELECT attempt.attempt_number, attempt.run_id, delivery.disposition,
                        attempt.state, run.state, run.outcome, run.failure
                   FROM mailbox_deliveries AS delivery
                   JOIN mailbox_delivery_attempts AS attempt ON attempt.event_id = delivery.event_id
                   JOIN runs AS run ON run.id = attempt.run_id
                  WHERE delivery.event_id = $1
                  ORDER BY attempt.attempt_number",
        )
        .bind(accepted.event_id.as_uuid())
        .fetch_all(&pool)
        .await
        .expect("load mailbox timeout evidence");
        eprintln!("golden mailbox timeout: evidence={evidence:?}");
    }
    let mailbox_run_id =
        mailbox_completion.expect("mailbox delivery reaches cleaned-up guest result");
    if let Some(fixture) = brokered_fixture.take() {
        fixture.upstream.assert_substituted_request().await;
    }
    assert_eq!(
        sqlx::query_scalar::<_, String>(
            "SELECT state_access_outcome FROM mailbox_delivery_attempts WHERE run_id = $1"
        )
        .bind(mailbox_run_id)
        .fetch_one(&pool)
        .await
        .expect("mailbox state access evidence"),
        "completed_access"
    );

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

fn caddy_configuration(admin_url: &str) -> Vec<u8> {
    let admin = reqwest::Url::parse(admin_url).expect("Caddy admin URL");
    let admin_listen = format!(
        "{}:{}",
        admin.host_str().expect("Caddy admin host"),
        admin.port().expect("Caddy admin port")
    );
    serde_json::json!({
        "admin": { "listen": admin_listen },
        "apps": { "http": { "servers": { "shared": {
            "listen": [env::var("HEPHAESTUS_CADDY_TEST_LISTEN").expect("joined Caddy listen address")],
            "routes": [
                { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform-owned" }] },
                { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                { "handle": [{ "handler": "static_response", "status_code": 404 }] }
            ]
        } } } }
    })
    .to_string()
    .into_bytes()
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
  image = { key = "golden-root" }
  triggers = ["refs/heads/main"]
  [build.resources]
  vcpus = 1
  memory_mib = 512
  [build.network]
  profile = "disabled"
  [[build.artifacts]]
  path = "bin/golden"
  kind = "executable"
  [guest]
  image = { key = "golden-root" }
  command = "bin/golden"
  arguments = []
  working_directory = "bin"
  [resources]
  vcpus = 1
  memory_mib = 512
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
  memory_mib = 512
  "#
}

#[allow(clippy::too_many_lines)]
async fn seed_reusable_instance(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    artifact_root: &Path,
    brokered_https: bool,
) -> SeededInstance {
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
    let cooking_artifact = cooking::agent_artifact();
    let artifact = cooking_artifact
        .as_deref()
        .unwrap_or(GOLDEN_AGENT.as_bytes());
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

    let secret_slot_schema = if cooking::enabled() {
        cooking::secret_slots()
    } else if brokered_https {
        serde_json::json!([{
            "key": "model",
            "purpose": "Call a fixture HTTPS API",
            "required": true,
            "delivery_modes": ["brokered"],
            "phases": ["normal"],
            "destinations": ["api.example.test"]
        }])
    } else {
        serde_json::json!([])
    };
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
                   '[]', $6, true, $7)",
    )
    .bind(release_agent_id)
    .bind(release_id)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/golden",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "root_image_digest": ROOT_IMAGE,
        "requires_state": true,
        "policy_ceiling": {
            "vcpus": 1,
            "memory_mib": 512,
            "network": if brokered_https { "broker_only" } else { "disabled" }
        }
    }))
    .bind([4_u8; 32].as_slice())
    .bind(secret_slot_schema)
    .bind(serde_json::json!({
        "command": "bin/golden",
        "arguments": [],
        "timeout_seconds": 60,
        "resources": {"vcpus": 1, "memory_mib": 512}
    }))
    .execute(pool)
    .await
    .expect("seed reusable release agent");
    sqlx::query(
        "INSERT INTO agent_instances
           (id, project_id, family_id, name, state, run_gate_open, created_by)
           VALUES ($1, $2, $3, 'golden-agent', 'active', false, $4)",
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
           VALUES ($1, $2, $3, $9, $4, '[]', $5, $6, $5, $7,
                   'platform/v1', true, '[]', $8)",
    )
    .bind(revision_id)
    .bind(instance_id)
    .bind(release_agent_id)
    .bind([5_u8; 32].as_slice())
    .bind(serde_json::json!({
        "vcpus": 1,
        "memory_mib": 512,
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind(serde_json::json!({
        "network": if brokered_https { "broker_only" } else { "disabled" }
    }))
    .bind([6_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(cooking::parameters())
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
    SeededInstance {
        instance: instance_id,
        revision: revision_id,
        attachment: attachment_id,
        release: release_id,
        release_agent: release_agent_id,
    }
}

struct SeededInstance {
    instance: uuid::Uuid,
    revision: uuid::Uuid,
    attachment: uuid::Uuid,
    release: uuid::Uuid,
    release_agent: uuid::Uuid,
}

/// Exact host-side identity retained by the joined Caddy/libkrun mailbox
/// proof. The released guest never receives this identifier.
struct GatewayGoldenFixture {
    mailbox_id: MailboxId,
    grant_id: uuid::Uuid,
}

/// Adds an exact released, stateless gateway handler alongside the reusable
/// agent. The artifact delegates only to the guest integration checker, which
/// validates that the daemon replaced the inbound secret before VM delivery.
async fn seed_gateway_release_agent(
    pool: &sqlx::PgPool,
    instance: &SeededInstance,
    artifact_root: &Path,
) -> uuid::Uuid {
    let agent_id = uuid::Uuid::new_v4();
    let artifact_id = uuid::Uuid::new_v4();
    let storage_key = uuid::Uuid::new_v4();
    let cooking_artifact = cooking::gateway_artifact();
    let artifact = cooking_artifact
        .as_deref()
        .unwrap_or(GATEWAY_HANDLER.as_bytes());
    let artifact_path = artifact_root.join(storage_key.simple().to_string());
    tokio::fs::write(&artifact_path, artifact)
        .await
        .expect("gateway release artifact");
    let mut permissions = tokio::fs::metadata(&artifact_path)
        .await
        .expect("gateway artifact metadata")
        .permissions();
    std::os::unix::fs::PermissionsExt::set_mode(&mut permissions, 0o555);
    tokio::fs::set_permissions(&artifact_path, permissions)
        .await
        .expect("gateway artifact mode");
    let artifact_hash: [u8; 32] = Sha256::digest(artifact).into();
    sqlx::query(
        "INSERT INTO release_artifacts
           (id, release_id, path, kind, mode, content_hash, size_bytes,
            media_type, storage_key)
         VALUES ($1, $2, 'bin/gateway', 'executable', 365, $3, $4,
                 'application/octet-stream', $5)",
    )
    .bind(artifact_id)
    .bind(instance.release)
    .bind(artifact_hash.as_slice())
    .bind(i64::try_from(artifact.len()).expect("gateway artifact length"))
    .bind(storage_key)
    .execute(pool)
    .await
    .expect("seed gateway release artifact");
    let family_id: uuid::Uuid =
        sqlx::query_scalar("SELECT family_id FROM release_agents WHERE id = $1")
            .bind(instance.release_agent)
            .fetch_one(pool)
            .await
            .expect("load reusable release family");
    sqlx::query(
        "INSERT INTO release_agents
           (id, release_id, family_id, agent_key, display_name,
            runtime_contract, runtime_contract_hash, parameter_schema,
            secret_slot_schema, requires_state, update_hook)
         VALUES ($1, $2, $3, 'golden-gateway', 'Golden gateway', $4, $5,
                 '[]', '[]', false, NULL)",
    )
    .bind(agent_id)
    .bind(instance.release)
    .bind(family_id)
    .bind(serde_json::json!({
        "executable": "bin/gateway",
        "arguments": [],
        "working_directory": "bin",
        "image_reference": ROOT_IMAGE,
        "requires_state": false,
        "policy_ceiling": {"vcpus": 1, "memory_mib": 512, "network": "disabled"}
    }))
    .bind([7_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed stateless gateway release agent");
    agent_id
}

/// Seeds immutable gateway route and host-only inbound secret authority.
// The fixture deliberately names each persisted authority boundary explicitly.
// Keeping this one setup transaction-shaped makes the golden authority graph
// reviewable without hiding a bound mailbox or grant in a generic helper.
#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
async fn seed_gateway_brokered_route(
    pool: &sqlx::PgPool,
    actor: UserId,
    project_id: uuid::Uuid,
    repository_id: uuid::Uuid,
    release_id: uuid::Uuid,
    release_agent_id: uuid::Uuid,
    import_id: uuid::Uuid,
    version_id: uuid::Uuid,
    mailbox_id: MailboxId,
) -> GatewayGoldenFixture {
    let gateway_id = uuid::Uuid::new_v4();
    let revision_id = uuid::Uuid::new_v4();
    let route_id = uuid::Uuid::new_v4();
    let binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateways
           (id, project_id, repository_id, name, lifecycle, created_by)
         VALUES ($1, $2, $3, 'golden-gateway', 'enabled', $4)",
    )
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("seed gateway");
    sqlx::query(
        "INSERT INTO gateway_revisions
           (id, gateway_id, project_id, repository_id, release_id, release_agent_id,
            release_agent_key, handler_contract, exposure, parameters, secret_slots, mailbox_slots,
            normalized_hash, created_by)
         VALUES ($1, $2, $3, $4, $5, $6, 'golden-gateway', 'http.v1', 'public',
                 $9, ARRAY['webhook'], $10, $7, $8)",
    )
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(repository_id)
    .bind(release_id)
    .bind(release_agent_id)
    .bind([8_u8; 32].as_slice())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        serde_json::json!({"inbound_placeholder":format!("heph-placeholder:v1:{version_id}"), "alice_provider_id":1001, "bob_provider_id":1002})
    } else {
        serde_json::json!({})
    })
    .bind(vec![if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    }])
    .execute(pool)
    .await
    .expect("seed immutable gateway revision");
    sqlx::query(
        "INSERT INTO gateway_routes
           (id, gateway_revision_id, gateway_id, project_id, path, methods)
         VALUES ($1, $2, $3, $4, $5, ARRAY['POST'])",
    )
    .bind(route_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(if cooking::enabled() {
        "/cooking/telegram"
    } else {
        "/brokered"
    })
    .execute(pool)
    .await
    .expect("seed gateway route");
    sqlx::query("UPDATE gateways SET active_revision_id = $2 WHERE id = $1")
        .bind(gateway_id)
        .bind(revision_id)
        .execute(pool)
        .await
        .expect("activate gateway revision");
    sqlx::query(
        "INSERT INTO gateway_secret_bindings
           (id, gateway_id, gateway_revision_id, import_id, slot_key,
            secret_version_id, status, normalized_hash)
         VALUES ($1, $2, $3, $4, 'webhook', $5, 'active', $6)",
    )
    .bind(binding_id)
    .bind(gateway_id)
    .bind(revision_id)
    .bind(import_id)
    .bind(version_id)
    .bind([9_u8; 32].as_slice())
    .execute(pool)
    .await
    .expect("seed gateway secret binding");
    let mailbox_binding_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_bindings
           (id, gateway_revision_id, gateway_id, project_id, slot_key, mailbox_id,
            producer_id, created_by)
         VALUES ($1, $2, $3, $4, $7, $5, 'golden-gateway', $6)",
    )
    .bind(mailbox_binding_id)
    .bind(revision_id)
    .bind(gateway_id)
    .bind(project_id)
    .bind(mailbox_id.as_uuid())
    .bind(actor.as_uuid())
    .bind(if cooking::enabled() {
        "cooking_requests"
    } else {
        "deliver"
    })
    .execute(pool)
    .await
    .expect("bind gateway fixture mailbox");
    let grant_id = uuid::Uuid::new_v4();
    sqlx::query(
        "INSERT INTO gateway_mailbox_binding_grants (id, binding_id, status, granted_by)
         VALUES ($1, $2, 'active', $3)",
    )
    .bind(grant_id)
    .bind(mailbox_binding_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("grant gateway fixture mailbox publication");
    sqlx::query(
        "INSERT INTO gateway_brokered_secret_rules
           (id, binding_id, gateway_revision_id, gateway_route_id, header_name, normalized_hash)
         VALUES ($1, $2, $3, $4, $6, $5)",
    )
    .bind(uuid::Uuid::new_v4())
    .bind(binding_id)
    .bind(revision_id)
    .bind(route_id)
    .bind([10_u8; 32].as_slice())
    .bind(if cooking::enabled() {
        "x-telegram-bot-api-secret-token"
    } else {
        "x-webhook-secret"
    })
    .execute(pool)
    .await
    .expect("seed gateway brokered inbound rule");
    GatewayGoldenFixture {
        mailbox_id,
        grant_id,
    }
}

/// Creates the complete durable secret authority that the daemon resolves at
/// mailbox dispatch. The only plaintext in this test is passed directly into
/// the encrypted secret-store boundary and is then asserted only at the local
/// TLS upstream.
#[allow(clippy::too_many_lines)]
async fn seed_brokered_https_fixture(
    pool: &sqlx::PgPool,
    actor: UserId,
    organization_id: OrganizationId,
    project_id: uuid::Uuid,
    instance: &SeededInstance,
) -> BrokeredFixture {
    sqlx::query(
        "INSERT INTO project_secret_roles (project_id, user_id, role)
          VALUES ($1, $2, 'secret_manager')",
    )
    .bind(project_id)
    .bind(actor.as_uuid())
    .execute(pool)
    .await
    .expect("authorize golden owner to manage project secret imports");

    let identity = AuthenticatedIdentity::new(
        actor,
        ISSUER,
        String::from("golden-subject"),
        serde_json::json!({}),
        RequestId::new(),
    );
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("golden secret key"),
        ),
        Arc::new(PostgresMelangeAuthorizer),
    );
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            &identity,
            CreateSecret {
                command_key: secret_command_key("create", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization_id),
                name: SecretName::parse(format!("golden_broker_{secret_id}"))
                    .expect("fixture secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(BROKERED_E2E_SENTINEL).expect("fixture secret value"),
            },
        )
        .await
        .expect("create encrypted brokered fixture secret");
    let grant_id = SecretGrantId::new();
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            &identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("grant-accept", import_id.as_uuid()),
                grant_id,
                secret_id,
                target: SecretTarget::Project(ProjectId::from_uuid(project_id)),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.example.test")],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("golden_broker").expect("fixture import alias"),
            },
        )
        .await
        .expect("grant and import brokered fixture secret");
    let binding_id = AgentSecretBindingId::new();
    let bound_revision = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            &identity,
            BindSecret {
                command_key: secret_command_key("bind", binding_id.as_uuid()),
                binding_id,
                instance_id: release_domain::AgentInstanceId::from_uuid(instance.instance),
                expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(
                    instance.revision,
                ),
                new_revision_id: bound_revision,
                import_id,
                slot: SecretSlotKey::parse("model").expect("fixture slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![instance.attachment],
                destinations: vec![String::from("api.example.test")],
            },
        )
        .await
        .expect("bind brokered fixture secret to immutable release revision");
    service
        .declare_brokered_https_rule(
            &identity,
            DeclareBrokeredHttpsRule {
                command_key: secret_command_key("declare-rule", BROKERED_E2E_RULE_ID),
                rule_id: BROKERED_E2E_RULE_ID,
                binding_id,
                destination: String::from("https://api.example.test"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare immutable brokered HTTPS fixture rule");
    let rule = BrokeredSecretRule {
        id: BrokeredSecretRuleId::from_uuid(BROKERED_E2E_RULE_ID),
        binding_id: binding_id.as_uuid(),
        instance_revision_id: bound_revision.as_uuid(),
        secret_version_id: version_id.as_uuid(),
        destination: Some(
            ExactHttpsOrigin::parse("https://api.example.test").expect("fixture destination"),
        ),
        location: HttpInjectionLocation::OutboundHeaderPrefix {
            header: HeaderName::parse("authorization").expect("fixture header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    };
    BrokeredFixture {
        upstream: BrokeredTlsUpstream::start(rule).await,
        import_id: import_id.as_uuid(),
        version_id: version_id.as_uuid(),
    }
}

struct BrokeredFixture {
    upstream: BrokeredTlsUpstream,
    import_id: uuid::Uuid,
    version_id: uuid::Uuid,
}

fn secret_command_key(operation: &str, id: uuid::Uuid) -> SecretCommandKey {
    SecretCommandKey::derive(operation, &[id.as_bytes()])
}

/// Certificate-valid loopback upstream available only to this daemon golden.
/// Its local address is accepted solely through `secret-broker`'s feature-gated
/// test fixture; the production registry still rejects private pins.
struct BrokeredTlsUpstream {
    adapter: Arc<dyn secret_application::BrokerAdapter>,
    observed: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl BrokeredTlsUpstream {
    async fn start(rule: BrokeredSecretRule) -> Self {
        let _already_installed = rustls::crypto::ring::default_provider().install_default();
        let mut ca_parameters = rcgen::CertificateParams::default();
        ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().expect("generate golden upstream CA key");
        let ca = ca_parameters
            .self_signed(&ca_key)
            .expect("self-sign golden upstream CA");
        let leaf_parameters = rcgen::CertificateParams::new(vec![String::from("api.example.test")])
            .expect("golden upstream DNS identity");
        let leaf_key = rcgen::KeyPair::generate().expect("generate golden upstream leaf key");
        let leaf = leaf_parameters
            .signed_by(&leaf_key, &ca, &ca_key)
            .expect("sign golden upstream leaf");
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
            )
            .expect("golden upstream TLS configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind golden TLS upstream");
        let port = listener
            .local_addr()
            .expect("golden TLS upstream address")
            .port();
        let observed = Arc::new(AtomicBool::new(false));
        let observed_server = Arc::clone(&observed);
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept golden TLS request");
            let mut stream = TlsAcceptor::from(Arc::new(tls))
                .accept(stream)
                .await
                .expect("verify golden TLS handshake");
            let mut request = Vec::with_capacity(512);
            loop {
                let mut chunk = [0_u8; 512];
                let read = stream.read(&mut chunk).await.expect("read golden request");
                assert_ne!(read, 0, "golden TLS request ended before headers");
                request.extend_from_slice(&chunk[..read]);
                assert!(
                    request.len() <= 8 * 1024,
                    "golden TLS request exceeded bound"
                );
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = std::str::from_utf8(&request).expect("golden HTTPS request is UTF-8");
            assert!(request.starts_with("GET /v1/probe HTTP/1.1\r\n"));
            assert!(request.contains(&format!(
                "authorization: Bearer {BROKERED_E2E_SENTINEL}\r\n"
            )));
            assert!(!request.contains("heph-placeholder:"));
            observed_server.store(true, Ordering::SeqCst);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\nbrokered-e2e-ok")
                .await
                .expect("respond to golden brokered request");
        });
        let adapter = BrokeredHttpsAdapterRegistry::test_only_local_trusted(
            rule,
            port,
            "127.0.0.1".parse().expect("golden loopback pin"),
            ca.pem().as_bytes(),
        )
        .expect("configure locally trusted pinned broker registry");
        Self {
            adapter: Arc::new(adapter),
            observed,
            server,
        }
    }

    fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    async fn assert_substituted_request(self) {
        tokio::time::timeout(Duration::from_secs(10), self.server)
            .await
            .expect("golden TLS upstream request timeout")
            .expect("golden TLS upstream task");
        assert!(
            self.observed.load(Ordering::SeqCst),
            "upstream did not receive the substituted HTTPS request"
        );
    }
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

fn mkfs_ext4() -> PathBuf {
    if let Some(path) = env::var_os("HEPHAESTUS_MKFS_EXT4") {
        return PathBuf::from(path);
    }
    ["/usr/sbin/mkfs.ext4", "/usr/bin/mkfs.ext4"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .expect("mkfs.ext4 must be installed for the golden volume fixture")
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
