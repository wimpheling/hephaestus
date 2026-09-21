//! First composed acceptance for the ordinary session-chat release.
//!
//! This module is included by the production golden daemon test. It keeps the
//! session-specific source, Git, instance, and broker assertions together while
//! reusing the golden PostgreSQL/NATS/libkrun fixture.

use super::secret_command_key;
use forge_domain::{GitRef, OrganizationId, ProjectId};
use forge_postgres::PgForgeRepository;
use forge_service::CreateRepository;
use hephaestus_app::RunningHephaestus;
use identity_domain::AuthenticatedIdentity;
use rpc_proto::{
    connect::hephaestus::{
        instance::v1::AgentInstanceServiceClient, release::v1::ReleaseServiceClient,
    },
    messages::hephaestus::{
        common::v1::{
            OpaqueId, ParameterValue, RequestContext, RuntimePolicy, parameter_value::Value,
        },
        instance::v1::{
            CapabilityBindingSelection, CreateAttachmentRequest, ImportAgentRequest, RefSelector,
            ReviseCapabilitiesRequest, TriggerPolicy, ref_selector,
        },
        release::v1::{InstallUiRequest, UiInstallationLifecycle, ui_installation_target},
    },
};
use secret_application::{
    BindSecret, CreateSecret, DeclareBrokeredHttpsRule, GrantAndAcceptSecretImport,
};
use secret_broker::BrokeredHttpsAdapterRegistry;
use secret_domain::{
    AgentSecretBindingId, DeliveryMode, ExecutionPhase, SecretAlias, SecretGrantId, SecretId,
    SecretImportId, SecretName, SecretOwner, SecretSlotKey, SecretTarget, SecretUsePolicy,
    SecretValue, SecretVersionId,
};
use secret_postgres::SecretService;
use secret_store::{EncryptedStore, LocalKeyProvider};
use serde_json::Value as JsonValue;
use sqlx::PgPool;
use std::{
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

pub const MODEL_RULE: Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000007");
const MODEL_SECRET_VALUE: &str = "session-chat-model-fixture-sentinel-007";

const MODEL_RESPONSE_TEXT: &str = "reference answer from deterministic model";
const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const HUMAN_RECORD_ID: &str = "22222222-2222-4222-8222-222222222222";

pub fn enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_E2E").as_deref() == Ok("1")
}

pub struct SessionBrokerFixture {
    adapter: Arc<dyn secret_application::BrokerAdapter>,
    observed: Arc<AtomicBool>,
    server: tokio::task::JoinHandle<()>,
}

impl SessionBrokerFixture {
    pub fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    async fn assert_observed(self) {
        tokio::time::timeout(Duration::from_secs(20), self.server)
            .await
            .expect("session-chat model upstream request timeout")
            .expect("session-chat model upstream task");
        assert!(self.observed.load(Ordering::SeqCst));
    }
}

/// Starts the CA-pinned deterministic HTTPS model before the daemon starts so
/// the production secret broker owns the only adapter used by the VM.
pub async fn start_model_fixture() -> SessionBrokerFixture {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let mut ca_parameters = rcgen::CertificateParams::default();
    ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().expect("session model CA key");
    let ca = ca_parameters
        .self_signed(&ca_key)
        .expect("session model CA");
    let leaf_key = rcgen::KeyPair::generate().expect("session model leaf key");
    let leaf = rcgen::CertificateParams::new(vec![String::from("api.model.example")])
        .expect("session model leaf name")
        .signed_by(&leaf_key, &ca, &ca_key)
        .expect("session model leaf");
    let tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
            rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
        )
        .expect("session model TLS configuration");
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("session model listener");
    let port = listener.local_addr().expect("session model address").port();
    let observed = Arc::new(AtomicBool::new(false));
    let observed_server = Arc::clone(&observed);
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.expect("session model connection");
        let mut stream = TlsAcceptor::from(Arc::new(tls))
            .accept(stream)
            .await
            .expect("session model TLS handshake");
        let request = read_http_request(&mut stream).await;
        let separator = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .expect("session model HTTP headers");
        let headers =
            std::str::from_utf8(&request[..separator]).expect("session model headers UTF-8");
        assert!(headers.starts_with("POST /v1/chat HTTP/1.1\r\n"));
        assert!(
            headers.contains("authorization: Bearer session-chat-model-fixture-sentinel-007\r\n")
        );
        assert!(!headers.contains("heph-placeholder:"));
        let body: JsonValue =
            serde_json::from_slice(&request[separator + 4..]).expect("session model body");
        assert!(
            body["messages"]
                .as_array()
                .is_some_and(|messages| !messages.is_empty())
        );
        assert_eq!(
            body["idempotency_key"],
            format!("{SESSION_ID}:{HUMAN_RECORD_ID}")
        );
        observed_server.store(true, Ordering::SeqCst);
        let body = br#"{"text":"reference answer from deterministic model"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("session model response headers");
        stream
            .write_all(body)
            .await
            .expect("session model response body");
    });
    let rule = super::BrokeredSecretRule {
        id: super::BrokeredSecretRuleId::from_uuid(MODEL_RULE),
        binding_id: Uuid::from_u128(8),
        instance_revision_id: Uuid::from_u128(9),
        secret_version_id: Uuid::from_u128(10),
        destination: Some(
            super::ExactHttpsOrigin::parse("https://api.model.example")
                .expect("session model origin"),
        ),
        location: super::HttpInjectionLocation::OutboundHeaderPrefix {
            header: super::HeaderName::parse("authorization").expect("session model header"),
            prefix: String::from("Bearer "),
        },
        gateway_route_id: None,
    };
    let adapter = BrokeredHttpsAdapterRegistry::test_only_local_trusted(
        rule,
        port,
        "127.0.0.1".parse().expect("session model loopback"),
        ca.pem().as_bytes(),
    )
    .expect("session model broker adapter");
    SessionBrokerFixture {
        adapter: Arc::new(adapter),
        observed,
        server,
    }
}

async fn read_http_request(
    stream: &mut tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
) -> Vec<u8> {
    let mut request = Vec::with_capacity(1024);
    loop {
        let mut chunk = [0_u8; 1024];
        let read = stream
            .read(&mut chunk)
            .await
            .expect("session model request read");
        assert_ne!(read, 0, "session model request ended early");
        request.extend_from_slice(&chunk[..read]);
        assert!(
            request.len() <= 64 * 1024,
            "session model request exceeded bound"
        );
        if let Some(separator) = request.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers =
                std::str::from_utf8(&request[..separator]).expect("session model request headers");
            let content_length = headers.lines().find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("content length"))
            });
            let body_start = separator + 4;
            if request.len() >= body_start + content_length.expect("content length") {
                return request;
            }
        }
    }
}

#[derive(Debug, sqlx::FromRow)]
struct RunDiagnostic {
    state: String,
    outcome: Option<String>,
    failure: Option<String>,
    exit_code: Option<i32>,
    exit_signal: Option<i32>,
}

// Keep the bounded diagnostic payload in one place so failures remain useful
// before the composed daemon and database are torn down.
#[allow(clippy::too_many_lines)]
async fn run_diagnostics(pool: &PgPool, run_id: Uuid) -> String {
    let run: Option<RunDiagnostic> = sqlx::query_as(
        "SELECT state, outcome, failure, exit_code, exit_signal
               FROM runs WHERE id = $1",
    )
    .bind(run_id)
    .fetch_optional(pool)
    .await
    .expect("read run outcome for diagnostics");
    let events: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT sequence, event_type,
                payload->'exit'->>'code', payload->'exit'->>'signal'
           FROM run_events
         WHERE run_id = $1 ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read run events for diagnostics");
    let log_chunks: Vec<JsonValue> = sqlx::query_scalar(
        "SELECT payload->'bytes'
           FROM run_events
          WHERE run_id = $1
            AND event_type = 'vm.log'
          ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read session-chat log diagnostics");
    let mut log_bytes = Vec::new();
    for chunk in log_chunks {
        let Some(values) = chunk.as_array() else {
            continue;
        };
        log_bytes.extend(
            values
                .iter()
                .filter_map(JsonValue::as_u64)
                .filter_map(|value| u8::try_from(value).ok()),
        );
    }
    let safe_agent_failures: Vec<String> = String::from_utf8_lossy(&log_bytes)
        .lines()
        .filter_map(|line| line.strip_prefix("session-chat agent failed: "))
        .filter_map(|failure| {
            let fields: Vec<&str> = failure.split_whitespace().collect();
            if fields.len() == 4 && fields[0] == "git" {
                let operation = fields[1].strip_prefix("operation=")?;
                let reason = fields[2].strip_prefix("reason=")?;
                let returncode = fields[3].strip_prefix("returncode=")?.parse::<i32>().ok()?;
                let operations = [
                    "add",
                    "clone",
                    "commit",
                    "config",
                    "diff_tree",
                    "fetch",
                    "init",
                    "ls_remote",
                    "merge",
                    "push",
                    "rebase",
                    "remote",
                    "rev_list",
                    "rev_parse",
                    "rm",
                    "show",
                    "symbolic_ref",
                    "other",
                ];
                let reasons = [
                    "auth",
                    "command_failed",
                    "credential_missing",
                    "invalidrepo",
                    "missinghelper",
                    "helper_action",
                    "helper_expected_host",
                    "helper_expected_path",
                    "helper_credential_path",
                    "helper_target",
                    "helper_authority_path",
                    "helper_authority_file",
                    "helper_authority_protection",
                    "helper_authority_decode",
                    "helper_credential_missing",
                    "helper_credential_length",
                    "helper_credential_encoding",
                    "helper_internal",
                    "nonfastforward",
                    "permission",
                    "rejected",
                    "unsafeownership",
                ];
                if operations.contains(&operation)
                    && reasons.contains(&reason)
                    && (-128..=255).contains(&returncode)
                {
                    return Some(format!(
                        "git operation={operation} reason={reason} returncode={returncode}"
                    ));
                }
                return None;
            }
            let stage = failure.trim();
            (!stage.is_empty()
                && stage
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'))
            .then(|| stage.to_owned())
        })
        .collect();
    format!(
        "run={run_id} state={} outcome={:?} failure={:?} exit_code={:?} exit_signal={:?} event_types={events:?} safe_agent_failures={safe_agent_failures:?}",
        run.as_ref().map_or("missing", |value| value.state.as_str()),
        run.as_ref().and_then(|value| value.outcome.as_deref()),
        run.as_ref().and_then(|value| value.failure.as_deref()),
        run.as_ref().and_then(|value| value.exit_code),
        run.as_ref().and_then(|value| value.exit_signal),
    )
}

async fn wait_for_run_succeeded(pool: &PgPool, run_id: Uuid, timeout: Duration) {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        let state: Option<String> = sqlx::query_scalar("SELECT state FROM runs WHERE id = $1")
            .bind(run_id)
            .fetch_optional(pool)
            .await
            .expect("read session-chat run state");
        let succeeded: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_events
                  WHERE run_id = $1 AND event_type = 'run.succeeded'
             )",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("read session-chat success event");
        let failed_event: bool = sqlx::query_scalar(
            "SELECT EXISTS(
                 SELECT 1 FROM run_events
                  WHERE run_id = $1 AND event_type IN ('run.failed', 'run.cancelled')
             )",
        )
        .bind(run_id)
        .fetch_one(pool)
        .await
        .expect("read session-chat failure event");
        if succeeded {
            return;
        }
        assert!(
            !(failed_event || matches!(state.as_deref(), Some("failed" | "cancelled"))),
            "session-chat run terminated before run.succeeded: {}",
            run_diagnostics(pool, run_id).await
        );
        // Keep this diagnostic before the caller tears down the daemon and
        // database; event payloads are deliberately excluded from output.
        assert!(
            tokio::time::Instant::now() < deadline,
            "session-chat run.succeeded timeout: {}",
            run_diagnostics(pool, run_id).await
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// Keep the cross-store provenance assertions together so a failed acceptance
// identifies one incomplete runtime turn rather than hiding it in helpers.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn assert_runtime_git_turn(
    pool: &PgPool,
    root: &Path,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    run_id: Uuid,
    human_commit: &str,
    expected_human_record_id: Option<&str>,
) {
    let human_request_attachment: Uuid = sqlx::query_scalar(
        "SELECT request.attachment_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
           JOIN git_receives AS receive ON receive.id = update.receive_id
          WHERE request.run_id = $1
            AND request.instance_id = $2
            AND request.repository_id = $3
            AND request.commit_sha = $4
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $5
            AND receive.repository_id = $3
            AND receive.actor_id = $6
            AND receive.status = 'accepted'
            AND receive.runtime_session_id IS NULL
            AND receive.runtime_attachment_id IS NULL
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $4
          LIMIT 1",
    )
    .bind(run_id)
    .bind(instance_id)
    .bind(repository_id)
    .bind(human_commit)
    .bind(attachment_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
    .expect("exact human Git receive/run provenance");
    assert_eq!(human_request_attachment, attachment_id);

    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;

    let agent_commit =
        git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{agent_commit}^")]
        )
        .await,
        human_commit,
        "canonical agent commit must be directly based on the human input commit"
    );
    let changed = git_output_bare(
        root,
        repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            &agent_commit,
        ],
    )
    .await;
    let paths = changed.lines().collect::<Vec<_>>();
    assert!(
        !paths.is_empty(),
        "agent commit must publish session output"
    );
    assert!(paths.iter().all(|path| {
        path.starts_with(".heph/session/v1/records/agent/agent%3Areference-chat/")
            || path.starts_with(".heph/session/v1/context/agent%3Areference-chat/")
    }));

    let agent_record_paths = paths
        .iter()
        .copied()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .collect::<Vec<_>>();
    assert_eq!(
        agent_record_paths.len(),
        1,
        "agent commit must publish exactly one assistant response"
    );
    let context_paths = paths
        .iter()
        .copied()
        .filter(|path| path.starts_with(".heph/session/v1/context/"))
        .collect::<Vec<_>>();
    assert_eq!(
        context_paths,
        [".heph/session/v1/context/agent%3Areference-chat/last_response.json"],
        "agent commit must publish exactly the response context entry"
    );

    let human_paths = git_output_bare(
        root,
        repository_id,
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "-r",
            human_commit,
        ],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        1,
        "human commit must publish exactly one canonical input record"
    );
    let human_record_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{human_commit}:{}", human_paths[0])],
    )
    .await;
    let human_record: JsonValue =
        serde_json::from_str(&human_record_json).expect("canonical human record JSON");
    let human_record_id = human_record["record_id"]
        .as_str()
        .expect("canonical human record ID");
    assert_eq!(
        human_record_id,
        human_paths[0]
            .strip_prefix(".heph/session/v1/records/human/")
            .and_then(|path| path.strip_suffix(".json"))
            .expect("canonical human record path")
            .replace("%3A", ":")
    );
    if let Some(expected_human_record_id) = expected_human_record_id {
        assert_eq!(human_record_id, expected_human_record_id);
    }

    let agent_record_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{agent_commit}:{}", agent_record_paths[0])],
    )
    .await;
    let agent_record: JsonValue =
        serde_json::from_str(&agent_record_json).expect("canonical agent record JSON");
    assert_eq!(agent_record["protocol"], "heph.session-chat");
    assert_eq!(agent_record["version"], 1);
    assert_eq!(agent_record["kind"], "assistant_message");
    assert_eq!(agent_record["actor"]["id"], "agent:reference-chat");
    assert_eq!(agent_record["actor"]["role"], "agent");
    assert_eq!(agent_record["participant_id"], "agent:reference-chat");
    assert_eq!(agent_record["in_reply_to"], human_record_id);
    assert_eq!(agent_record["content"]["kind"], "text");
    assert_eq!(agent_record["content"]["text"], MODEL_RESPONSE_TEXT);
    Uuid::parse_str(
        agent_record["correlation_id"]
            .as_str()
            .expect("canonical agent correlation ID"),
    )
    .expect("canonical agent correlation UUID");

    let context_json = git_output_bare(
        root,
        repository_id,
        &["show", &format!("{agent_commit}:{}", context_paths[0])],
    )
    .await;
    let context: JsonValue = serde_json::from_str(&context_json).expect("canonical context JSON");
    assert_eq!(context["agent_id"], "agent:reference-chat");
    assert_eq!(context["key"], "last_response");
    assert_eq!(context["value"], MODEL_RESPONSE_TEXT);

    let (runtime_receive_id, runtime_session_id, runtime_attachment_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT receive.id, receive.runtime_session_id, receive.runtime_attachment_id
               FROM git_ref_updates AS update
               JOIN git_receives AS receive ON receive.id = update.receive_id
              WHERE receive.repository_id = $1
                AND receive.status = 'accepted'
                AND receive.runtime_session_id IS NOT NULL
                AND receive.runtime_attachment_id IS NOT NULL
                AND update.git_ref = 'refs/heads/main'
                AND update.old_commit = $2
                AND update.new_commit = $3
              LIMIT 1",
        )
        .bind(repository_id)
        .bind(human_commit)
        .bind(&agent_commit)
        .fetch_one(pool)
        .await
        .expect("runtime-authenticated agent Git receive provenance");
    assert_eq!(runtime_attachment_id, attachment_id);

    let (session_run_id, session_instance_id, session_attachment_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT session.run_id, session.instance_id, session.attachment_id
               FROM runtime_authority_sessions AS session
              WHERE session.id = $1
                AND session.run_id = $2
                AND session.instance_id = $3
                AND session.attachment_id IS NOT NULL",
        )
        .bind(runtime_session_id)
        .bind(run_id)
        .bind(instance_id)
        .fetch_one(pool)
        .await
        .expect("runtime session immutable provenance");
    assert_eq!(session_run_id, run_id);
    assert_eq!(session_instance_id, instance_id);
    assert_eq!(session_attachment_id, attachment_id);
    let (provenance_revision, run_revision, target_repository, target_ref, target_commit): (
        Uuid,
        Uuid,
        Uuid,
        String,
        String,
    ) = sqlx::query_as(
        "SELECT provenance.instance_revision_id, run.instance_revision_id
                , provenance.target_repository_id, provenance.target_ref
                , provenance.target_commit
           FROM run_instance_provenance AS provenance
           JOIN runs AS run ON run.id = provenance.run_id
          WHERE provenance.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("immutable run revision provenance");
    assert_eq!(provenance_revision, run_revision);
    assert_eq!(target_repository, repository_id);
    assert_eq!(target_ref, "refs/heads/main");
    assert_eq!(target_commit, human_commit);
    let runtime_request_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM run_requests WHERE receive_id = $1")
            .bind(runtime_receive_id)
            .fetch_one(pool)
            .await
            .expect("runtime originating receive request count");
    assert_eq!(
        runtime_request_count, 0,
        "originating runtime attachment must not recursively trigger itself"
    );
}

/// Builds, installs, triggers, and verifies one session-chat turn through the
/// production Git/build/release/instance/runtime boundaries.
// The composed acceptance intentionally owns the complete production setup
// and both Git paths; splitting it would obscure the exact scenario boundary.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub async fn exercise(
    pool: &PgPool,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    repositories: &PgForgeRepository,
    identity: &AuthenticatedIdentity,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    broker: SessionBrokerFixture,
    timeout: Duration,
) {
    let build_context = super::cooking_builds::CookingBuildContext {
        pool,
        running,
        root,
        source_root,
        project_id: project,
        repositories,
        identity: super::cooking_builds::CookingIdentity {
            actor: identity,
            git_token,
            rpc_token,
        },
        timeout,
    };
    let built = super::cooking_builds::build_one(
        &build_context,
        "session-chat",
        source_root.to_path_buf(),
        "Reference session chat agent",
        false,
    )
    .await
    .expect("build and publish session-chat release through production workers");
    assert!(!built.release_id.is_nil());
    assert!(!built.release_agent_id.is_nil());
    let browser_e2e =
        std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1");

    let session_repository = repositories
        .create_repository_trusted(&CreateRepository {
            project_id: project,
            name: format!("session-chat-{}", Uuid::new_v4()),
            default_branch: GitRef::parse("refs/heads/main").expect("session main ref"),
            is_public: false,
            agent_runs_enabled: true,
        })
        .await
        .expect("create session repository");
    let (checkout, baseline) = if browser_e2e {
        (None, None)
    } else {
        let checkout = root.join("session-chat-checkout");
        initialize_session_checkout(
            &checkout,
            source_root,
            session_repository.id.as_uuid(),
            git_token,
            running,
            &format!("user:{}", identity.user_id),
        )
        .await;
        let baseline = git_output(&checkout, &["rev-parse", "HEAD"]).await;
        (Some(checkout), Some(baseline))
    };

    let imported = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ImportAgent",
    )
    .expect("session ImportAgent client")
    .import_agent(ImportAgentRequest {
        context: mutation_context("session-chat-import").into(),
        project_id: opaque(project.as_uuid()).into(),
        release_agent_id: opaque(built.release_agent_id).into(),
        name: String::from("reference-session-chat"),
        parameters: vec![ParameterValue {
            name: String::from("model_rule_id"),
            value: Some(Value::StringValue(MODEL_RULE.to_string())),
            ..Default::default()
        }],
        selected_policy: RuntimePolicy {
            vcpus: 1,
            memory_mib: 256,
            network: rpc_proto::messages::hephaestus::common::v1::NetworkPolicy::BrokerOnly.into(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
    .await
    .expect("ImportAgent session-chat release")
    .into_owned();
    let instance_id = response_id(imported.instance_id.into_option(), "session instance")
        .expect("session instance ID");
    let revision_id = response_id(imported.revision_id.into_option(), "session revision")
        .expect("session revision ID");
    let attachment = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/CreateAttachment",
    )
    .expect("session CreateAttachment client")
    .create_attachment(CreateAttachmentRequest {
        context: mutation_context("session-chat-attachment").into(),
        instance_id: opaque(instance_id).into(),
        repository_id: opaque(session_repository.id.as_uuid()).into(),
        ref_selector: RefSelector {
            selector: Some(ref_selector::Selector::Exact(String::from(
                "refs/heads/main",
            ))),
            ..Default::default()
        }
        .into(),
        trigger_policy: TriggerPolicy::Push.into(),
        ..Default::default()
    })
    .await
    .expect("CreateAttachment session-chat repository")
    .into_owned();
    let attachment_id = response_id(attachment.attachment_id.into_option(), "session attachment")
        .expect("session attachment ID");

    // Capability delegation is a separate project authority from ordinary
    // project maintenance. Seed the fixture role, then exercise the
    // production ReviseCapabilities RPC below to create the immutable
    // capability revision.
    sqlx::query(
        "INSERT INTO project_capability_granters (project_id, user_id, created_by)
         VALUES ($1, $2, $2)
         ON CONFLICT (project_id, user_id) DO NOTHING",
    )
    .bind(project.as_uuid())
    .bind(identity.user_id.as_uuid())
    .execute(pool)
    .await
    .expect("session capability delegation role");

    let revised = instance_client(
        running,
        rpc_token,
        "/hephaestus.instance.v1.AgentInstanceService/ReviseCapabilities",
    )
    .expect("session ReviseCapabilities client")
    .revise_capabilities(ReviseCapabilitiesRequest {
        context: mutation_context("session-chat-capabilities").into(),
        instance_id: opaque(instance_id).into(),
        expected_revision_id: opaque(revision_id).into(),
        bindings: vec![CapabilityBindingSelection {
            slot_key: String::from("session"),
            resource_kind: String::from("repository"),
            resource_id: opaque(session_repository.id.as_uuid()).into(),
            granted_operations: vec![String::from("git_read"), String::from("update_ref")],
            ..Default::default()
        }],
        ..Default::default()
    })
    .await
    .expect("revise session capability")
    .into_owned();
    assert!(
        !revised.runnable,
        "the capability revision must remain gated on the required secret"
    );
    let capability_revision_id = response_id(
        revised.instance_revision_id.into_option(),
        "session capability revision",
    )
    .expect("session capability revision ID");
    let _active_revision_id = seed_model_secret(
        pool,
        organization,
        project,
        identity,
        instance_id,
        capability_revision_id,
        attachment_id,
    )
    .await;

    if browser_e2e {
        let installation = install_session_chat_ui(
            running,
            rpc_token,
            organization,
            session_repository.id.as_uuid(),
            built.release_id,
        )
        .await;
        run_session_chat_browser(
            running,
            project,
            session_repository.id.as_uuid(),
            installation,
            identity.user_id.as_uuid(),
        )
        .await;
        // Browser initialization can itself create an idle run. Derive the
        // human input from the parent of the canonical agent commit, then
        // resolve that exact receive into its durable run request.
        let agent_commit = git_output_bare(
            root,
            session_repository.id.as_uuid(),
            &["rev-parse", "refs/heads/main"],
        )
        .await;
        let human_commit = git_output_bare(
            root,
            session_repository.id.as_uuid(),
            &["rev-parse", &format!("{agent_commit}^")],
        )
        .await;
        let browser_run_id: Uuid = sqlx::query_scalar(
            "SELECT request.run_id
               FROM run_requests AS request
               JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
              WHERE request.instance_id = $1
                AND request.repository_id = $2
                AND request.commit_sha = $3
                AND request.git_ref = 'refs/heads/main'
                AND request.request_kind = 'instance_normal'
                AND request.attachment_id = $4
                AND update.git_ref = 'refs/heads/main'
                AND update.new_commit = $3
              LIMIT 1",
        )
        .bind(instance_id)
        .bind(session_repository.id.as_uuid())
        .bind(&human_commit)
        .bind(attachment_id)
        .fetch_one(pool)
        .await
        .expect("session-chat browser human-input run");
        assert_runtime_git_turn(
            pool,
            root,
            session_repository.id.as_uuid(),
            instance_id,
            attachment_id,
            identity.user_id.as_uuid(),
            browser_run_id,
            &human_commit,
            None,
        )
        .await;
        broker.assert_observed().await;
        return;
    }

    let user_id = identity.user_id.to_string();
    let checkout = checkout.expect("standalone session checkout");
    let baseline = baseline.expect("standalone session baseline");
    append_human_and_push(
        &checkout,
        source_root,
        git_token,
        running,
        &user_id,
        baseline.as_str(),
    )
    .await;
    let human_commit = git_output(&checkout, &["rev-parse", "HEAD"]).await;
    let run_id: Uuid = sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $4
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $3
          LIMIT 1",
    )
    .bind(session_repository.id.as_uuid())
    .bind(instance_id)
    .bind(&human_commit)
    .bind(attachment_id)
    .fetch_one(pool)
    .await
    .expect("session-chat push run request");
    assert_runtime_git_turn(
        pool,
        root,
        session_repository.id.as_uuid(),
        instance_id,
        attachment_id,
        identity.user_id.as_uuid(),
        run_id,
        &human_commit,
        Some(HUMAN_RECORD_ID),
    )
    .await;
    let (stored_input, stored_ref, stored_revision): (String, String, Uuid) = sqlx::query_as(
        "SELECT request.commit_sha, request.git_ref, run.instance_revision_id
           FROM run_requests request JOIN runs run ON run.id = request.run_id
          WHERE request.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("session-chat runtime provenance");
    assert_eq!(stored_input, human_commit);
    assert_eq!(stored_ref, "refs/heads/main");
    assert_ne!(stored_revision, revision_id);
    tokio::time::sleep(Duration::from_secs(2)).await;
    let run_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(session_repository.id.as_uuid())
    .fetch_one(pool)
    .await
    .expect("session-chat run request count");
    assert_eq!(
        run_count, 1,
        "assistant publication must not recursively trigger a run"
    );
    broker.assert_observed().await;
}

async fn install_session_chat_ui(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    organization: OrganizationId,
    repository: Uuid,
    release_id: Uuid,
) -> (Uuid, Uuid) {
    let uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("session UI release RPC URI");
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!(
                "Bearer {}",
                token_factory("/hephaestus.release.v1.ReleaseService/InstallUi")
            ))
            .expect("session UI release RPC authorization"),
        )
        .with_default_timeout(Duration::from_secs(30));
    let client = ReleaseServiceClient::new(connectrpc::client::HttpClient::plaintext(), config);
    let target = rpc_proto::messages::hephaestus::release::v1::UiInstallationTarget {
        target: Some(ui_installation_target::Target::RepositoryId(
            opaque(repository).into(),
        )),
        ..Default::default()
    };
    let response = client
        .install_ui(InstallUiRequest {
            context: mutation_context("session-chat-ui-install").into(),
            organization_id: opaque(organization.as_uuid()).into(),
            target: target.into(),
            release_id: opaque(release_id).into(),
            ui_key: String::from("session-chat"),
            acknowledge_repository_git_access: true,
            ..Default::default()
        })
        .await
        .expect("InstallUi session-chat release")
        .into_owned();
    assert_eq!(
        response.lifecycle.to_i32(),
        UiInstallationLifecycle::UI_INSTALLATION_LIFECYCLE_ENABLED as i32,
        "session-chat UI installation must be enabled"
    );
    (
        response
            .installation_id
            .into_option()
            .expect("session-chat UI installation ID")
            .value
            .parse()
            .expect("session-chat UI installation UUID"),
        response
            .generation_id
            .into_option()
            .expect("session-chat UI generation ID")
            .value
            .parse()
            .expect("session-chat UI generation UUID"),
    )
}

async fn run_session_chat_browser(
    running: &RunningHephaestus,
    project: ProjectId,
    repository: Uuid,
    installation: (Uuid, Uuid),
    actor: Uuid,
) {
    let fixture_path = std::env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
        .expect("session-chat browser fixture output path");
    let fixture = serde_json::json!({
        "session_chat_ui": {
            "project_id": project,
            "repository_id": repository,
            "installation_id": installation.0,
            "generation_id": installation.1,
            "actor_id": actor,
            "ui_path": "/session-chat/index.html",
            "agent_response_text": MODEL_RESPONSE_TEXT
        }
    });
    tokio::fs::write(
        &fixture_path,
        serde_json::to_vec_pretty(&fixture).expect("session-chat browser fixture JSON"),
    )
    .await
    .expect("write session-chat browser fixture JSON");
    let control_dir = Path::new(&fixture_path)
        .parent()
        .expect("session-chat fixture parent")
        .join("installed-ui-control");
    tokio::fs::create_dir(&control_dir)
        .await
        .expect("create session-chat installed UI control directory");
    std::fs::set_permissions(
        &control_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("session-chat installed UI control directory mode");
    let issuer = std::env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
        .expect("session-chat browser OIDC issuer");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-installed-ui-e2e.sh");
    let status = Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &fixture_path)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL",
            std::env::var("HEPHAESTUS_POSTGRES_TEST_URL")
                .expect("session-chat browser database URL"),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            running.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", "initial")
        .env(
            "HEPHAESTUS_INSTALLED_UI_BROWSER_GREP",
            "cooking session-chat installed UI",
        )
        .env(
            "HEPHAESTUS_PLATFORM_HTTPS_ORIGIN",
            super::installed_ui_platform_origin(),
        )
        .env("HEPHAESTUS_UI_NAMESPACE", super::installed_ui_namespace())
        .env(
            "HEPHAESTUS_CADDY_TEST_CA_CERT",
            std::env::var("HEPHAESTUS_CADDY_TEST_CA_CERT")
                .expect("session-chat browser Caddy CA certificate"),
        )
        .status()
        .await
        .expect("run session-chat installed UI browser E2E");
    assert!(
        status.success(),
        "session-chat browser E2E failed: {status}"
    );
}

async fn initialize_session_checkout(
    checkout: &Path,
    source_root: &Path,
    repository: Uuid,
    git_token: &str,
    running: &RunningHephaestus,
    human_id: &str,
) {
    let initializer = r"
from pathlib import Path
import sys
from git_adapter import LocalGitSession
LocalGitSession.initialize(Path(sys.argv[1]), sys.argv[2], human_ids=(sys.argv[3],))
";
    let output = Command::new("python3")
        .arg("-c")
        .arg(initializer)
        .arg(checkout)
        .arg(SESSION_ID)
        .arg(human_id)
        .env("PYTHONPATH", source_root)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("initialize session checkout");
    assert!(
        output.status.success(),
        "session initialization failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let remote = format!("http://{}/{}", running.http_addr(), repository);
    git(checkout, &["remote", "add", "origin", &remote]).await;
    authenticated_git(
        checkout,
        git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await;
}

async fn append_human_and_push(
    checkout: &Path,
    source_root: &Path,
    git_token: &str,
    _running: &RunningHephaestus,
    user_id: &str,
    baseline: &str,
) {
    let appender = r#"
from pathlib import Path
import sys
from git_adapter import LocalGitSession
from protocol import Record, TextContent, utc_now
session = LocalGitSession.open(Path(sys.argv[1]))
actor = "user:" + sys.argv[4]
session.append_human(Record(sys.argv[3], "user_message", actor, "human", actor, utc_now(), content=TextContent("hello session")), sys.argv[2])
"#;
    let output = Command::new("python3")
        .arg("-c")
        .arg(appender)
        .arg(checkout)
        .arg(baseline)
        .arg(HUMAN_RECORD_ID)
        .arg(user_id)
        .env("PYTHONPATH", source_root)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("append session human record");
    assert!(
        output.status.success(),
        "session human append failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    authenticated_git(
        checkout,
        git_token,
        &["push", "origin", "HEAD:refs/heads/main"],
    )
    .await;
}

async fn seed_model_secret(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
    instance: Uuid,
    revision: Uuid,
    attachment: Uuid,
) -> Uuid {
    sqlx::query("INSERT INTO project_secret_roles (project_id,user_id,role) VALUES ($1,$2,'secret_manager')")
        .bind(project.as_uuid())
        .bind(identity.user_id.as_uuid())
        .execute(pool)
        .await
        .expect("session secret manager role");
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("session secret key"),
        ),
        Arc::new(super::PostgresMelangeAuthorizer),
    );
    let secret_id = SecretId::new();
    let version_id = SecretVersionId::new();
    service
        .create(
            identity,
            CreateSecret {
                command_key: secret_command_key("session-chat-create", secret_id.as_uuid()),
                secret_id,
                version_id,
                owner: SecretOwner::Organization(organization),
                name: SecretName::parse(format!("session_chat_{secret_id}"))
                    .expect("session secret name"),
                allowed_delivery_modes: vec![DeliveryMode::Brokered],
                value: SecretValue::new(MODEL_SECRET_VALUE).expect("session secret value"),
            },
        )
        .await
        .expect("create session model secret");
    let import_id = SecretImportId::new();
    service
        .grant_and_accept_import(
            identity,
            GrantAndAcceptSecretImport {
                command_key: secret_command_key("session-chat-grant", import_id.as_uuid()),
                grant_id: SecretGrantId::new(),
                secret_id,
                target: SecretTarget::Project(project),
                policy: SecretUsePolicy {
                    delivery_modes: vec![DeliveryMode::Brokered],
                    phases: vec![ExecutionPhase::Normal],
                    destinations: vec![String::from("api.model.example")],
                },
                expires_at: None,
                import_id,
                alias: SecretAlias::parse("model").expect("session model alias"),
            },
        )
        .await
        .expect("grant session model secret");
    let binding_id = AgentSecretBindingId::new();
    let bound_revision = release_domain::AgentInstanceRevisionId::new();
    service
        .bind_secret(
            identity,
            BindSecret {
                command_key: secret_command_key("session-chat-bind", binding_id.as_uuid()),
                binding_id,
                instance_id: release_domain::AgentInstanceId::from_uuid(instance),
                expected_revision_id: release_domain::AgentInstanceRevisionId::from_uuid(revision),
                new_revision_id: bound_revision,
                import_id,
                slot: SecretSlotKey::parse("model").expect("session model slot"),
                mode: DeliveryMode::Brokered,
                phases: vec![ExecutionPhase::Normal],
                attachment_ids: vec![attachment],
                destinations: vec![String::from("api.model.example")],
            },
        )
        .await
        .expect("bind session model secret");
    service
        .declare_brokered_https_rule(
            identity,
            DeclareBrokeredHttpsRule {
                command_key: secret_command_key("session-chat-rule", MODEL_RULE),
                rule_id: MODEL_RULE,
                binding_id,
                destination: String::from("https://api.model.example"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare session model rule");
    bound_revision.as_uuid()
}

fn instance_client(
    running: &RunningHephaestus,
    token_factory: &(dyn Fn(&str) -> String + Send + Sync),
    audience: &str,
) -> Result<
    AgentInstanceServiceClient<connectrpc::client::HttpClient>,
    Box<dyn std::error::Error + Send + Sync>,
> {
    let uri = format!("http://{}", running.http_addr()).parse()?;
    let config = connectrpc::client::ClientConfig::new(uri)
        .with_default_header(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", token_factory(audience)))?,
        )
        .with_default_timeout(Duration::from_secs(30));
    Ok(AgentInstanceServiceClient::new(
        connectrpc::client::HttpClient::plaintext(),
        config,
    ))
}

fn mutation_context(operation: &str) -> RequestContext {
    RequestContext {
        request_id: opaque(Uuid::new_v4()).into(),
        idempotency_key: format!("session-chat-{operation}-{}", Uuid::new_v4()),
        ..Default::default()
    }
}

fn opaque(value: Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

fn response_id(
    value: Option<OpaqueId>,
    operation: &str,
) -> Result<Uuid, Box<dyn std::error::Error + Send + Sync>> {
    value
        .ok_or_else(|| format!("{operation} returned no ID"))?
        .value
        .parse()
        .map_err(Into::into)
}

async fn git(directory: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run session Git command");
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
        .stdin(Stdio::null())
        .output()
        .await
        .expect("run authenticated session Git command");
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
        .expect("run session Git output command");
    assert!(
        output.status.success(),
        "Git output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("session Git output UTF-8")
        .trim()
        .to_owned()
}

async fn git_output_bare(root: &Path, repository: Uuid, arguments: &[&str]) -> String {
    let bare = root.join("repositories").join(format!("{repository}.git"));
    let output = Command::new("git")
        .arg(format!("--git-dir={}", bare.display()))
        .args(arguments)
        .output()
        .await
        .expect("run session bare Git output command");
    assert!(
        output.status.success(),
        "bare Git output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("session bare Git output UTF-8")
        .trim()
        .to_owned()
}
