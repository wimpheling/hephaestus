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
    connect::hephaestus::instance::v1::AgentInstanceServiceClient,
    messages::hephaestus::{
        common::v1::{
            OpaqueId, ParameterValue, RequestContext, RuntimePolicy, parameter_value::Value,
        },
        instance::v1::{
            CapabilityBindingSelection, CreateAttachmentRequest, ImportAgentRequest, RefSelector,
            ReviseCapabilitiesRequest, TriggerPolicy, ref_selector,
        },
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
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs, io,
    os::unix::fs as unix_fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use time::OffsetDateTime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    process::Command,
};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;

pub const MODEL_RULE: Uuid = uuid::uuid!("00000000-0000-0000-0000-000000000007");
pub const MODEL_SECRET_VALUE: &str = "session-chat-model-fixture-sentinel-007";

const MODEL_RESPONSE_TEXT: &str = "reference answer from deterministic model";
const SESSION_ID: &str = "11111111-1111-4111-8111-111111111111";
const HUMAN_RECORD_ID: &str = "22222222-2222-4222-8222-222222222222";
const DENIAL_HUMAN_RECORD_ID: &str = "55555555-5555-4555-8555-555555555555";

#[path = "fork.rs"]
mod fork;

pub fn enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_E2E").as_deref() == Ok("1")
}

pub fn denial_probe_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_DENIAL_PROBE_E2E").as_deref() == Ok("1")
}

pub fn restart_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_RESTART_E2E").as_deref() == Ok("1")
}

pub fn concurrent_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_CONCURRENT_E2E").as_deref() == Ok("1")
}

pub fn fork_e2e_enabled() -> bool {
    std::env::var("HEPHAESTUS_APP_SESSION_CHAT_FORK_E2E").as_deref() == Ok("1")
}

pub struct SessionBrokerFixture {
    adapter: Arc<dyn secret_application::BrokerAdapter>,
    observed: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<ObservedModelRequest>>>,
    expected_requests: usize,
    server: tokio::task::JoinHandle<()>,
}

impl SessionBrokerFixture {
    pub fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    async fn assert_observed(self) -> Vec<ObservedModelRequest> {
        tokio::time::timeout(Duration::from_secs(20), self.server)
            .await
            .expect("session-chat model upstream request timeout")
            .expect("session-chat model upstream task");
        assert_eq!(
            self.observed.load(Ordering::SeqCst),
            self.expected_requests,
            "deterministic model must observe every expected turn"
        );
        Arc::try_unwrap(self.requests)
            .expect("session model request observer still shared")
            .into_inner()
            .expect("session model request observer lock")
    }

    async fn wait_for_observed(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while self.observed.load(Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("session-chat model prefix observation timeout");
    }

    fn observed_snapshot(&self) -> Vec<ObservedModelRequest> {
        self.requests
            .lock()
            .expect("session model request observer lock")
            .clone()
    }
}

#[derive(Clone, Debug)]
struct ObservedModelRequest {
    session_id: Uuid,
    record_id: Uuid,
    messages: Vec<ObservedModelMessage>,
}

#[derive(Clone, Debug)]
struct ObservedModelMessage {
    record_id: Uuid,
    role: String,
}

/// Starts the CA-pinned deterministic HTTPS model before the daemon starts so
/// the production secret broker owns the only adapter used by the VM.
#[allow(clippy::too_many_lines)] // The bounded TLS fixture keeps request validation in one place.
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
    let tls = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
            )
            .expect("session model TLS configuration"),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("session model listener");
    let port = listener.local_addr().expect("session model address").port();
    if concurrent_e2e_enabled() {
        assert_eq!(
            std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref(),
            Ok("1"),
            "session-chat concurrency E2E requires the browser phase"
        );
        assert!(
            restart_e2e_enabled(),
            "session-chat concurrency E2E requires the restart phase"
        );
    }
    if fork_e2e_enabled() {
        assert_eq!(
            std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref(),
            Ok("1"),
            "session-chat fork E2E requires the browser phase"
        );
        assert!(
            restart_e2e_enabled() && concurrent_e2e_enabled(),
            "session-chat fork E2E requires restart and concurrency phases"
        );
    }
    let expected_requests = if fork_e2e_enabled() {
        6
    } else if concurrent_e2e_enabled() {
        5
    } else if restart_e2e_enabled() {
        3
    } else if std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1")
        || denial_probe_enabled()
    {
        2
    } else {
        1
    };
    let observed = Arc::new(AtomicUsize::new(0));
    let observed_server = Arc::clone(&observed);
    let requests = Arc::new(Mutex::new(Vec::with_capacity(expected_requests)));
    let requests_server = Arc::clone(&requests);
    let server = tokio::spawn(async move {
        let mut keys = HashSet::new();
        for request_index in 0..expected_requests {
            let (stream, _) = listener.accept().await.expect("session model connection");
            let mut stream = TlsAcceptor::from(Arc::clone(&tls))
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
                headers
                    .contains("authorization: Bearer session-chat-model-fixture-sentinel-007\r\n")
            );
            assert!(!headers.contains("heph-placeholder:"));
            let body: JsonValue =
                serde_json::from_slice(&request[separator + 4..]).expect("session model body");
            assert!(
                body["messages"]
                    .as_array()
                    .is_some_and(|messages| !messages.is_empty())
            );
            let key = body["idempotency_key"]
                .as_str()
                .expect("session model idempotency key");
            let (session_id, record_id) = key
                .split_once(':')
                .expect("session model idempotency key shape");
            Uuid::parse_str(session_id).expect("session model session UUID");
            Uuid::parse_str(record_id).expect("session model record UUID");
            if expected_requests == 1 {
                assert_eq!(key, format!("{SESSION_ID}:{HUMAN_RECORD_ID}"));
            }
            assert!(
                keys.insert(key.to_owned()),
                "model turn keys must be distinct"
            );
            let messages = body["messages"]
                .as_array()
                .expect("session model messages")
                .iter()
                .map(|message| ObservedModelMessage {
                    record_id: Uuid::parse_str(
                        message["record_id"]
                            .as_str()
                            .expect("session model message record ID"),
                    )
                    .expect("session model message record UUID"),
                    role: message["role"]
                        .as_str()
                        .expect("session model message role")
                        .to_owned(),
                })
                .collect();
            requests_server
                .lock()
                .expect("session model request observer lock")
                .push(ObservedModelRequest {
                    session_id: Uuid::parse_str(session_id).expect("session model session UUID"),
                    record_id: Uuid::parse_str(record_id).expect("session model record UUID"),
                    messages,
                });
            observed_server.store(request_index + 1, Ordering::SeqCst);
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
        }
        assert_eq!(keys.len(), expected_requests);
    });
    let adapter = BrokeredHttpsAdapterRegistry::test_only_local_origin_catalog(
        "https://api.model.example",
        port,
        "127.0.0.1".parse().expect("session model loopback"),
        ca.pem().as_bytes(),
    )
    .expect("session model broker adapter");
    SessionBrokerFixture {
        adapter: Arc::new(adapter),
        observed,
        requests,
        expected_requests,
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
    let denial_probe_markers: Vec<String> = String::from_utf8_lossy(&log_bytes)
        .lines()
        .filter_map(|line| {
            line.split_once("HEPH_SESSION_CHAT_DENIAL_PROBE ")?
                .1
                .strip_suffix('\r')
        })
        .filter(|marker| {
            let fields: Vec<&str> = marker.split_whitespace().collect();
            fields.len() == 2
                && fields[0].starts_with("check=")
                && DENIAL_PROBE_CHECKS
                    .iter()
                    .any(|check| fields[0] == format!("check={check}"))
                && matches!(fields[1], "status=passed" | "status=failed")
        })
        .map(str::to_owned)
        .collect();
    format!(
        "run={run_id} state={} outcome={:?} failure={:?} exit_code={:?} exit_signal={:?} event_types={events:?} safe_agent_failures={safe_agent_failures:?} denial_probe_markers={denial_probe_markers:?}",
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
    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
    let agent_commit =
        git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_runtime_git_turn_at_commit(
        pool,
        root,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        run_id,
        human_commit,
        &agent_commit,
        expected_human_record_id,
    )
    .await;
}

#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // This is one reviewable assertion over a runtime turn at a specified canonical commit.
async fn assert_runtime_git_turn_at_commit(
    pool: &PgPool,
    root: &Path,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    run_id: Uuid,
    human_commit: &str,
    agent_commit: &str,
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
            agent_commit,
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
        .bind(agent_commit)
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

const DENIAL_PROBE_CHECKS: [&str; 10] = [
    "source_checkout_absent",
    "model_authorized_control",
    "source_repository_read_denied",
    "source_repository_push_denied",
    "other_repository_read_denied",
    "other_repository_push_denied",
    "prohibited_path_push_denied",
    "model_destination_denied",
    "model_rule_denied",
    "runtime_git_credential_absent_from_surfaces",
];

async fn accepted_receive_count(pool: &PgPool, repository_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM git_receives
          WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("count accepted denial-probe Git receives")
}

async fn canonical_main_ref(pool: &PgPool, repository_id: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT commit_sha FROM git_refs
          WHERE repository_id = $1 AND git_ref = 'refs/heads/main'",
    )
    .bind(repository_id)
    .fetch_optional(pool)
    .await
    .expect("read denial-probe canonical main ref")
}

async fn assert_denial_probe_output(pool: &PgPool, run_id: Uuid) {
    let chunks: Vec<JsonValue> = sqlx::query_scalar(
        "SELECT payload->'bytes'
           FROM run_events
          WHERE run_id = $1 AND event_type = 'vm.log'
          ORDER BY sequence",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .expect("read denial-probe VM output");
    let mut bytes = Vec::new();
    for chunk in chunks {
        if let Some(values) = chunk.as_array() {
            bytes.extend(
                values
                    .iter()
                    .filter_map(JsonValue::as_u64)
                    .filter_map(|value| u8::try_from(value).ok()),
            );
        }
    }
    let output = String::from_utf8_lossy(&bytes);
    for check in DENIAL_PROBE_CHECKS {
        let marker = format!("HEPH_SESSION_CHAT_DENIAL_PROBE check={check} status=passed");
        assert_eq!(
            output.matches(&marker).count(),
            1,
            "denial probe must emit one passed marker for {check}"
        );
    }
    assert_eq!(
        output
            .matches("HEPH_SESSION_CHAT_DENIAL_PROBE check=")
            .count(),
        DENIAL_PROBE_CHECKS.len(),
        "denial probe must emit exactly ten fixed markers"
    );
}

fn copy_denial_probe_source(source: &Path, destination: &Path) -> io::Result<()> {
    if !source.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "session-chat denial source is not a directory",
        ));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == ".git" || name == "target" || name == "__pycache__" {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(&name);
        let metadata = fs::symlink_metadata(&source_path)?;
        if metadata.is_dir() {
            copy_denial_probe_source(&source_path, &destination_path)?;
        } else if metadata.file_type().is_symlink() {
            unix_fs::symlink(fs::read_link(source_path)?, destination_path)?;
        } else if metadata.is_file() {
            fs::copy(source_path, destination_path)?;
        } else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session-chat denial source contains unsupported file type",
            ));
        }
    }
    Ok(())
}

// Keep the generated guest entrypoint beside its source staging so the release
// fixture remains auditable as one bounded denial-probe setup.
#[allow(clippy::too_many_lines)]
fn prepare_denial_probe_source(source_root: &Path, root: &Path) -> PathBuf {
    let destination = root.join(format!("session-chat-denial-source-{}", Uuid::new_v4()));
    copy_denial_probe_source(source_root, &destination).expect("copy denial-probe source");
    let denied_probe_destination = destination.join("tests");
    fs::create_dir_all(&denied_probe_destination).expect("create denial-probe package");
    fs::copy(
        source_root.join("tests/denied_probe.py"),
        denied_probe_destination.join("denied_probe.py"),
    )
    .expect("stage denial-probe module");
    fs::write(
        destination.join("denied_probe_entry.py"),
        r#"#!/usr/local/bin/python3
import importlib.util
import json
from pathlib import Path
import sys

import agent

_PROBE_SPEC = importlib.util.spec_from_file_location(
    "session_chat_denied_probe", Path(__file__).with_name("tests") / "denied_probe.py"
)
if _PROBE_SPEC is None or _PROBE_SPEC.loader is None:
    raise SystemExit(1)
denied_probe = importlib.util.module_from_spec(_PROBE_SPEC)
_PROBE_SPEC.loader.exec_module(denied_probe)

def main():
    context = json.loads((Path("/run/hephaestus") / "context.json").read_text(encoding="utf-8"))
    parameters = json.loads((Path("/run/hephaestus") / "parameters.json").read_text(encoding="utf-8"))
    target = context.get("repository_id")
    source = parameters.get("denial_source_repository_id")
    other = parameters.get("denial_other_repository_id")
    if not all(isinstance(value, str) for value in (target, source, other)):
        return 1
    try:
        observer = denied_probe.runtime_credential_observer(target)
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        observer = None
    try:
        results = denied_probe._run(
            target,
            source,
            other,
            Path("/workspace/git"),
            observer,
        )
    except Exception:
        results = {check: False for check in denied_probe.CHECKS}
    final_check = denied_probe.FINAL_CHECK
    for check in denied_probe.CHECKS:
        if check == final_check:
            continue
        status = "passed" if results.get(check) is True else "failed"
        print(
            f"HEPH_SESSION_CHAT_DENIAL_PROBE check={check} status={status}",
            file=sys.stderr,
            flush=True,
        )
    failed = [
        index for index, check in enumerate(denied_probe.CHECKS)
        if check != final_check and results.get(check) is not True
    ]
    if failed:
        return 40 + failed[0]
    try:
        with denied_probe.observe_agent_git(observer):
            agent.run_once()
    except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
        results[final_check] = False
    else:
        try:
            results[final_check] = observer is not None and observer.scan_config(Path("/workspace/git"))
        except Exception:  # noqa: BLE001 - output must stay fixed and credential-free.
            results[final_check] = False
    status = "passed" if results[final_check] is True else "failed"
    print(
        f"HEPH_SESSION_CHAT_DENIAL_PROBE check={final_check} status={status}",
        file=sys.stderr,
        flush=True,
    )
    return 0 if results[final_check] is True else 49


if __name__ == "__main__":
    raise SystemExit(main())
"#,
    )
    .expect("write denial-probe entrypoint");
    let build_script = destination.join("build.sh");
    let mut build = fs::read_to_string(&build_script).expect("read session-chat build script");
    build.push_str(
        "\npython3 - <<'PY'\nfrom pathlib import Path\nfor source in (\"tests/denied_probe.py\", \"denied_probe_entry.py\"):\n    path = Path(source)\n    compile(path.read_text(encoding=\"utf-8\"), str(path), \"exec\", dont_inherit=True)\nPY\ninstall -m 0755 denied_probe_entry.py \"$output_root/bin/session-chat/session-chat-agent\"\ninstall -m 0644 agent.py \"$output_root/bin/session-chat/agent.py\"\ninstall -d -m 0755 \"$output_root/bin/session-chat/tests\"\ninstall -m 0644 tests/denied_probe.py \"$output_root/bin/session-chat/tests/denied_probe.py\"\n",
    );
    let agent_config = destination.join("agent.toml");
    let mut config = fs::read_to_string(&agent_config).expect("read denial-probe agent config");
    config.push_str(
        r#"

[[parameters]]
name = "denial_source_repository_id"
type = "string"
minimum_length = 36
maximum_length = 36
required = true

[[parameters]]
name = "denial_other_repository_id"
type = "string"
minimum_length = 36
maximum_length = 36
required = true
"#,
    );
    fs::write(agent_config, config).expect("write denial-probe agent config");
    fs::write(build_script, build).expect("write denial-probe build script");
    destination
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
pub async fn exercise<'a>(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &'a Path,
    project: ProjectId,
    organization: OrganizationId,
    repositories: &PgForgeRepository,
    identity: &'a AuthenticatedIdentity,
    git_token: &'a str,
    rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    broker: SessionBrokerFixture,
    timeout: Duration,
) -> Option<BrowserRestartState<'a>> {
    let denial_probe = denial_probe_enabled();
    let browser_e2e =
        std::env::var("HEPHAESTUS_APP_SESSION_CHAT_BROWSER_E2E").as_deref() == Ok("1");
    assert!(
        !concurrent_e2e_enabled() || (browser_e2e && restart_e2e_enabled()),
        "session-chat concurrency E2E requires browser and restart phases"
    );
    assert!(
        !fork_e2e_enabled() || (browser_e2e && restart_e2e_enabled() && concurrent_e2e_enabled()),
        "session-chat fork E2E requires browser, restart, and concurrency phases"
    );
    assert!(
        !denial_probe || !browser_e2e,
        "denial probe is a standalone session mode"
    );
    let (denial_other_repository_id, denial_source_root) = if denial_probe {
        let other = repositories
            .create_repository_trusted(&CreateRepository {
                project_id: project,
                name: format!("session-chat-denial-other-{}", Uuid::new_v4()),
                default_branch: GitRef::parse("refs/heads/main").expect("denial other ref"),
                is_public: false,
                agent_runs_enabled: false,
            })
            .await
            .expect("create denial-probe comparison repository");
        let other_id = other.id.as_uuid();
        let other_checkout = root.join("session-chat-denial-other-checkout");
        initialize_session_checkout(
            &other_checkout,
            source_root,
            other_id,
            git_token,
            running,
            &format!("user:{}", identity.user_id),
        )
        .await;
        let prepared = prepare_denial_probe_source(source_root, root);
        (Some(other_id), Some(prepared))
    } else {
        (None, None)
    };
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
        denial_source_root
            .clone()
            .unwrap_or_else(|| source_root.to_path_buf()),
        "Reference session chat agent",
        false,
    )
    .await
    .expect("build and publish session-chat release through production workers");
    assert!(!built.release_id.is_nil());
    assert!(!built.release_agent_id.is_nil());
    grant_session_capability(pool, project, identity).await;
    let denial_source_repository_id = denial_probe.then_some(built.repository_id.as_uuid());
    if browser_e2e {
        eprintln!("HEPH_SESSION_CHAT_BROWSER stage=branch-selected mode=session_chat_new");
        let existing_repository_ids: HashSet<Uuid> =
            sqlx::query_scalar("SELECT id FROM repositories WHERE project_id = $1")
                .bind(project.as_uuid())
                .fetch_all(pool)
                .await
                .expect("existing session browser repositories")
                .into_iter()
                .collect();
        let model_import = seed_model_import(pool, organization, project, identity).await;
        run_session_chat_browser(
            database_url,
            running,
            project,
            built.release_agent_id,
            model_import,
            SessionChatBrowserMode::New,
        )
        .await;
        if restart_e2e_enabled() {
            broker.wait_for_observed(2).await;
            let requests = broker.observed_snapshot();
            assert_browser_session(
                pool,
                root,
                project,
                identity.user_id.as_uuid(),
                built.release_agent_id,
                &existing_repository_ids,
                requests,
            )
            .await;
            let state = load_browser_restart_state(
                pool,
                root,
                project,
                organization,
                source_root,
                identity,
                git_token,
                rpc_token,
                identity.user_id.as_uuid(),
                built.release_id,
                built.release_agent_id,
                &existing_repository_ids,
                broker,
            )
            .await;
            eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-state-captured turns=2");
            return Some(state);
        }
        let requests = broker.assert_observed().await;
        assert_browser_session(
            pool,
            root,
            project,
            identity.user_id.as_uuid(),
            built.release_agent_id,
            &existing_repository_ids,
            requests,
        )
        .await;
        return None;
    }

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
    let mut parameters = vec![ParameterValue {
        name: String::from("model_rule_id"),
        value: Some(Value::StringValue(MODEL_RULE.to_string())),
        ..Default::default()
    }];
    if denial_probe {
        parameters.extend([
            ParameterValue {
                name: String::from("denial_source_repository_id"),
                value: Some(Value::StringValue(
                    denial_source_repository_id
                        .expect("denial source repository ID")
                        .to_string(),
                )),
                ..Default::default()
            },
            ParameterValue {
                name: String::from("denial_other_repository_id"),
                value: Some(Value::StringValue(
                    denial_other_repository_id
                        .expect("denial comparison repository ID")
                        .to_string(),
                )),
                ..Default::default()
            },
        ]);
    }

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
        parameters,
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

    let user_id = identity.user_id.to_string();
    let checkout = checkout.expect("standalone session checkout");
    let baseline = baseline.expect("standalone session baseline");
    let accepted_before_turn = accepted_receive_count(pool, session_repository.id.as_uuid()).await;
    let denial_source_ref_before =
        denial_source_repository_id.map(|repository_id| canonical_main_ref(pool, repository_id));
    let denial_source_ref_before = match denial_source_ref_before {
        Some(reference) => Some(reference.await),
        None => None,
    };
    let denial_other_ref_before =
        denial_other_repository_id.map(|repository_id| canonical_main_ref(pool, repository_id));
    let denial_other_ref_before = match denial_other_ref_before {
        Some(reference) => Some(reference.await),
        None => None,
    };
    let denial_source_accepts_before = denial_source_repository_id
        .map(|repository_id| accepted_receive_count(pool, repository_id));
    let denial_source_accepts_before = match denial_source_accepts_before {
        Some(count) => Some(count.await),
        None => None,
    };
    let denial_other_accepts_before =
        denial_other_repository_id.map(|repository_id| accepted_receive_count(pool, repository_id));
    let denial_other_accepts_before = match denial_other_accepts_before {
        Some(count) => Some(count.await),
        None => None,
    };
    append_human_and_push(
        &checkout,
        source_root,
        git_token,
        running,
        &user_id,
        baseline.as_str(),
        if denial_probe {
            DENIAL_HUMAN_RECORD_ID
        } else {
            HUMAN_RECORD_ID
        },
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
        Some(if denial_probe {
            DENIAL_HUMAN_RECORD_ID
        } else {
            HUMAN_RECORD_ID
        }),
    )
    .await;
    if let (
        Some(source_id),
        Some(other_id),
        Some(source_before),
        Some(other_before),
        Some(source_ref_before),
        Some(other_ref_before),
    ) = (
        denial_source_repository_id,
        denial_other_repository_id,
        denial_source_accepts_before,
        denial_other_accepts_before,
        denial_source_ref_before,
        denial_other_ref_before,
    ) {
        assert_eq!(
            source_ref_before.as_deref(),
            Some(built.source_commit.as_str()),
            "denial source must be the published build repository"
        );
        assert!(
            other_ref_before.is_some(),
            "comparison repository must have a canonical main ref"
        );
        assert_denial_probe_output(pool, run_id).await;
        assert_eq!(
            accepted_receive_count(pool, session_repository.id.as_uuid()).await,
            accepted_before_turn + 2,
            "denial pushes must not add an accepted target receive"
        );
        assert_eq!(
            accepted_receive_count(pool, source_id).await,
            source_before,
            "source-repository denial attempts must not be accepted"
        );
        assert_eq!(
            accepted_receive_count(pool, other_id).await,
            other_before,
            "other-repository denial attempts must not be accepted"
        );
        assert_eq!(
            canonical_main_ref(pool, source_id).await,
            source_ref_before,
            "source-repository canonical ref must remain unchanged"
        );
        assert_eq!(
            canonical_main_ref(pool, other_id).await,
            other_ref_before,
            "other-repository canonical ref must remain unchanged"
        );
        eprintln!(
            "HEPH_SESSION_CHAT_DENIAL_PROBE host=validated checks=10 refs=unchanged receives=unchanged"
        );
    }
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
    let observed = broker.assert_observed().await;
    if denial_probe {
        assert_eq!(
            observed
                .iter()
                .map(|request| request.session_id)
                .collect::<Vec<_>>(),
            [
                Uuid::parse_str(SESSION_ID).expect("denial-probe session UUID"),
                Uuid::parse_str(SESSION_ID).expect("denial-probe session UUID"),
            ]
        );
        assert_eq!(
            observed
                .iter()
                .map(|request| request.record_id)
                .collect::<Vec<_>>(),
            [
                Uuid::parse_str("22222222-2222-4222-8222-222222222222")
                    .expect("denial-probe control record UUID"),
                Uuid::parse_str(DENIAL_HUMAN_RECORD_ID).expect("denial-probe human record UUID"),
            ]
        );
    }
    None
}

async fn grant_session_capability(
    pool: &PgPool,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
) {
    // Browser OIDC resolves golden-subject to this same identity; the explicit
    // project grant authorizes the user-selected repository capability.
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
}

enum SessionChatBrowserMode {
    New,
    Existing {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
    },
    Concurrent {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
        initial_transcript_count: usize,
        initial_agent_count: usize,
    },
    Fork {
        repository_id: Uuid,
        installation_id: Uuid,
        generation_id: Uuid,
        actor_id: Uuid,
        initial_transcript_count: usize,
        initial_agent_count: usize,
    },
}

#[allow(clippy::too_many_lines)] // Browser setup and selector validation stay one bounded fixture boundary.
async fn run_session_chat_browser(
    database_url: &str,
    running: &RunningHephaestus,
    project: ProjectId,
    release_agent_id: Uuid,
    model_import_id: Uuid,
    mode: SessionChatBrowserMode,
) {
    let browser_phase = match &mode {
        SessionChatBrowserMode::New => "initial",
        SessionChatBrowserMode::Existing { .. } => "recovery",
        SessionChatBrowserMode::Concurrent { .. } => "concurrency",
        SessionChatBrowserMode::Fork { .. } => "fork",
    };
    let fixture_output = PathBuf::from(
        std::env::var("HEPHAESTUS_COOKING_BROWSER_FIXTURE_OUTPUT")
            .expect("session-chat browser fixture output path"),
    );
    let (fixture, browser_mode, browser_selector) = match mode {
        SessionChatBrowserMode::New => (
            serde_json::json!({
                "session_chat_new": {
                    "project_id": project,
                    "release_agent_id": release_agent_id,
                    "model_import_id": model_import_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT
                }
            }),
            "session_chat_new",
            "cooking new session chat creates and opens a real Git-backed browser session",
        ),
        SessionChatBrowserMode::Existing {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
        } => (
            serde_json::json!({
                "session_chat_ui": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": "4",
                    "initial_agent_count": "2"
                }
            }),
            "session_chat_ui",
            "cooking session-chat installed UI initializes and reconnects ordinary Git history",
        ),
        SessionChatBrowserMode::Concurrent {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count,
            initial_agent_count,
        } => (
            serde_json::json!({
                "session_chat_concurrent": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": initial_transcript_count.to_string(),
                    "initial_agent_count": initial_agent_count.to_string()
                }
            }),
            "session_chat_concurrent",
            "cooking concurrent session chat clients reconcile a stale Git push and preserve both turns",
        ),
        SessionChatBrowserMode::Fork {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count,
            initial_agent_count,
        } => (
            serde_json::json!({
                "session_chat_fork": {
                    "project_id": project,
                    "repository_id": repository_id,
                    "installation_id": installation_id,
                    "generation_id": generation_id,
                    "actor_id": actor_id,
                    "ui_path": "/session-chat/index.html",
                    "agent_response_text": MODEL_RESPONSE_TEXT,
                    "initial_transcript_count": initial_transcript_count.to_string(),
                    "initial_agent_count": initial_agent_count.to_string()
                }
            }),
            "session_chat_fork",
            "cooking forked session chat preserves inherited history and receives a fresh response",
        ),
    };
    let fixture_path = match browser_mode {
        "session_chat_new" => fixture_output,
        "session_chat_ui" => fixture_output.with_file_name(format!(
            "{}.recovery.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        "session_chat_concurrent" => fixture_output.with_file_name(format!(
            "{}.concurrency.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        "session_chat_fork" => fixture_output.with_file_name(format!(
            "{}.fork.json",
            fixture_output
                .file_name()
                .expect("session-chat fixture filename")
                .to_string_lossy()
        )),
        _ => unreachable!("session-chat browser fixture mode is allowlisted"),
    };
    eprintln!(
        "HEPH_SESSION_CHAT_BROWSER stage=fixture-selected mode={browser_mode} selector={browser_selector}"
    );
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
    match tokio::fs::create_dir(&control_dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            let metadata = tokio::fs::symlink_metadata(&control_dir)
                .await
                .expect("read existing session-chat installed UI control directory");
            assert!(
                metadata.is_dir() && !metadata.file_type().is_symlink(),
                "session-chat installed UI control directory must remain a real directory"
            );
        }
        Err(error) => panic!("create session-chat installed UI control directory: {error}"),
    }
    std::fs::set_permissions(
        &control_dir,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )
    .expect("session-chat installed UI control directory mode");
    let issuer = std::env::var("HEPHAESTUS_COOKING_BROWSER_OIDC_ISSUER")
        .expect("session-chat browser OIDC issuer");
    let script =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/run-installed-ui-e2e.sh");
    let browser_timer = super::WorkloadPhaseTimer::start(
        match browser_phase {
            "initial" => "browser-initial",
            "recovery" => "browser-recovery",
            "concurrency" => "browser-concurrency",
            "fork" => "browser-fork",
            _ => unreachable!("session-chat browser phase is allowlisted"),
        },
        super::workload_phase_timing_from_environment(),
    );
    let status = Command::new(script)
        .env("HEPHAESTUS_E2E_COOKING_FIXTURE", &fixture_path)
        .env("HEPHAESTUS_E2E_EXTERNAL_DATABASE_URL", database_url)
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_ENDPOINT",
            running.http_addr().to_string(),
        )
        .env(
            "HEPHAESTUS_E2E_EXTERNAL_RPC_SECRET",
            "golden-internal-command-token-with-sufficient-entropy",
        )
        .env("HEPHAESTUS_E2E_EXTERNAL_OIDC_ISSUER", issuer)
        .env("HEPHAESTUS_E2E_COOKING_PHASE", browser_phase)
        .env("HEPHAESTUS_INSTALLED_UI_BROWSER_GREP", browser_selector)
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
    browser_timer.finish(status.success());
    assert!(
        status.success(),
        "session-chat browser E2E failed: {status}"
    );
    eprintln!(
        "HEPH_SESSION_CHAT_BROWSER stage=playwright-passed mode={browser_mode} selector={browser_selector}"
    );
}

#[derive(Debug, sqlx::FromRow)]
#[allow(clippy::struct_field_names)] // These names mirror the four persisted relation IDs.
struct BrowserSessionObjects {
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
}

/// Persisted identifiers and immutable first-phase evidence used by recovery.
pub struct BrowserRestartState<'a> {
    broker: SessionBrokerFixture,
    project: ProjectId,
    organization: OrganizationId,
    source_root: &'a Path,
    identity: &'a AuthenticatedIdentity,
    git_token: &'a str,
    rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    actor_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    session_id: Uuid,
    initial_head: String,
    initial_receive_count: i64,
    initial_record_blobs: HashMap<String, Vec<u8>>,
    previous_runtime_session_ids: Vec<Uuid>,
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)] // One exact persisted-session lookup keeps restart identity immutable.
async fn load_browser_restart_state<'a>(
    pool: &PgPool,
    root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    source_root: &'a Path,
    identity: &'a AuthenticatedIdentity,
    git_token: &'a str,
    rpc_token: &'a (dyn Fn(&str) -> String + Send + Sync),
    actor_id: Uuid,
    release_id: Uuid,
    release_agent_id: Uuid,
    existing_repository_ids: &HashSet<Uuid>,
    broker: SessionBrokerFixture,
) -> BrowserRestartState<'a> {
    let candidates: Vec<BrowserSessionObjects> = sqlx::query_as(
        "SELECT repository.id AS repository_id,
                instance.id AS instance_id,
                revision.id AS revision_id,
                attachment.id AS attachment_id
           FROM repositories repository
           JOIN agent_attachments attachment
             ON attachment.repository_id = repository.id
            AND attachment.project_id = repository.project_id
           JOIN agent_instances instance
             ON instance.id = attachment.instance_id
            AND instance.project_id = repository.project_id
           JOIN agent_instance_revisions revision
             ON revision.id = instance.active_revision_id
            AND revision.instance_id = instance.id
            AND revision.release_agent_id = $2
          WHERE repository.project_id = $1
            AND attachment.ref_selector = 'refs/heads/main'
            AND attachment.removed_at IS NULL",
    )
    .bind(project.as_uuid())
    .bind(release_agent_id)
    .fetch_all(pool)
    .await
    .expect("browser restart repository and attachment");
    let candidates: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| !existing_repository_ids.contains(&candidate.repository_id))
        .collect();
    assert_eq!(
        candidates.len(),
        1,
        "browser restart must retain exactly one session repository"
    );
    let BrowserSessionObjects {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
    } = candidates
        .into_iter()
        .next()
        .expect("browser restart session repository");
    let (installation_id, generation_id): (Uuid, Uuid) = sqlx::query_as(
        "SELECT installation.id, installation.current_generation_id
           FROM ui_installations installation
           JOIN ui_installation_generations generation
             ON generation.id = installation.current_generation_id
            AND generation.installation_id = installation.id
          WHERE installation.project_id = $1
            AND installation.repository_id = $2
            AND installation.scope = 'repository'
            AND installation.ui_key = 'session-chat'
            AND installation.lifecycle = 'enabled'
            AND generation.release_id = $3
            AND generation.ui_key = 'session-chat'
            AND generation.ui_scope = 'repository'",
    )
    .bind(project.as_uuid())
    .bind(repository_id)
    .bind(release_id)
    .fetch_one(pool)
    .await
    .expect("browser restart installed UI generation");
    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    let manifest_path = ".heph/session/v1/manifest.json";
    let manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            repository_id,
            &["show", &format!("{head}:{manifest_path}")],
        )
        .await,
    )
    .expect("browser restart session manifest JSON");
    let session_id = Uuid::parse_str(
        manifest["data"]["session_id"]
            .as_str()
            .expect("browser restart session ID"),
    )
    .expect("browser restart session UUID");
    let initial_receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("browser restart initial receive count");
    let record_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", &head],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut initial_record_blobs = HashMap::with_capacity(record_paths.len());
    for path in record_paths {
        let content =
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await;
        initial_record_blobs.insert(path, content);
    }
    let previous_runtime_session_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT session.id
           FROM runtime_authority_sessions AS session
           JOIN run_requests AS request ON request.run_id = session.run_id
          WHERE session.instance_id = $1
            AND session.attachment_id = $2
            AND request.request_kind = 'instance_normal'
          ORDER BY session.created_at, session.id",
    )
    .bind(instance_id)
    .bind(attachment_id)
    .fetch_all(pool)
    .await
    .expect("browser restart previous runtime authority sessions");
    let previous_publication_session_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT DISTINCT session.id
           FROM runtime_authority_sessions AS session
           JOIN git_receives AS receive
             ON receive.runtime_session_id = session.id
          WHERE session.instance_id = $1
            AND session.attachment_id = $2
            AND receive.runtime_attachment_id = $2
            AND receive.status = 'accepted'
          ORDER BY session.id",
    )
    .bind(instance_id)
    .bind(attachment_id)
    .fetch_all(pool)
    .await
    .expect("browser restart previous publication runtime sessions");
    assert_eq!(
        previous_publication_session_ids.len(),
        2,
        "first browser phase must publish two runtime-authenticated turns"
    );
    BrowserRestartState {
        broker,
        project,
        organization,
        source_root,
        identity,
        git_token,
        rpc_token,
        actor_id,
        release_id,
        release_agent_id,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        installation_id,
        generation_id,
        session_id,
        initial_head: head,
        initial_receive_count,
        initial_record_blobs,
        previous_runtime_session_ids,
    }
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
/// Restarts the browser-backed session against the already-persisted installation.
pub async fn exercise_browser_restart(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    state: BrowserRestartState<'_>,
    restart_boundary: OffsetDateTime,
) {
    let BrowserRestartState {
        broker,
        project,
        organization,
        source_root,
        identity,
        git_token,
        rpc_token,
        actor_id,
        release_id,
        release_agent_id,
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
        installation_id,
        generation_id,
        session_id,
        initial_head,
        initial_receive_count,
        initial_record_blobs,
        previous_runtime_session_ids,
    } = state;
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-started");
    run_session_chat_browser(
        database_url,
        running,
        project,
        release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Existing {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
        },
    )
    .await;
    broker.wait_for_observed(3).await;
    let requests = broker.observed_snapshot();
    assert_eq!(
        requests.len(),
        3,
        "restart flow must make three model turns"
    );
    let first = &requests[0];
    let second = &requests[1];
    let third = &requests[2];
    assert_eq!(third.session_id, session_id);
    assert_eq!(third.messages.len(), 5);
    assert_eq!(third.messages[0].role, "user");
    assert_eq!(third.messages[1].role, "assistant");
    assert_eq!(third.messages[2].role, "user");
    assert_eq!(third.messages[3].role, "assistant");
    assert_eq!(third.messages[4].role, "user");
    assert_eq!(third.messages[0].record_id, first.record_id);
    assert_eq!(third.messages[2].record_id, second.record_id);
    assert_eq!(third.messages[4].record_id, third.record_id);
    assert_ne!(third.record_id, first.record_id);
    assert_ne!(third.record_id, second.record_id);

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(
        head, initial_head,
        "restart must persist a new canonical turn"
    );
    let ancestor = Command::new("git")
        .arg(format!(
            "--git-dir={}",
            root.join("repositories")
                .join(format!("{repository_id}.git"))
                .display()
        ))
        .args(["merge-base", "--is-ancestor", &initial_head, &head])
        .status()
        .await
        .expect("check restart Git ancestry");
    assert!(
        ancestor.success(),
        "restart head must retain the first two turns"
    );
    let paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", head.trim()],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let human_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .cloned()
        .collect::<Vec<_>>();
    let agent_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        3,
        "restart must retain three human records"
    );
    assert_eq!(
        agent_paths.len(),
        3,
        "restart must retain three assistant records"
    );
    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("restart human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("restart human record ID"),
        )
        .expect("restart human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_record_paths = HashMap::with_capacity(agent_paths.len());
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("restart assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("restart assistant record ID"),
        )
        .expect("restart assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, record);
    }
    let first_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == first.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart first assistant record");
    let second_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == second.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart second assistant record");
    assert_eq!(third.messages[1].record_id, first_agent_id);
    assert_eq!(third.messages[3].record_id, second_agent_id);
    let third_agent_id = agent_record_paths
        .iter()
        .find(|(_, record)| record["in_reply_to"] == third.record_id.to_string())
        .map(|(record_id, _)| *record_id)
        .expect("restart third assistant record");
    let human_path = human_record_paths
        .get(&third.record_id)
        .expect("restart third human record path");
    let human_commit = canonical_record_commit(root, repository_id, human_path).await;
    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{human_commit}^")],
        )
        .await,
        initial_head,
        "third human commit must directly follow the pre-restart head"
    );
    let agent_path = agent_paths
        .iter()
        .find(|path| path.ends_with(&format!("{third_agent_id}.json")))
        .expect("restart third assistant record path");
    let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
    assert_eq!(
        git_output_bare(
            root,
            repository_id,
            &["rev-parse", &format!("{agent_commit}^")],
        )
        .await,
        human_commit,
        "restart assistant commit must be based on its human commit"
    );
    let run_id = accepted_normal_run_id(
        pool,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        &human_commit,
    )
    .await;
    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
    assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
    let expected_human_record_id = third.record_id.to_string();
    assert_runtime_git_turn_at_commit(
        pool,
        root,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        run_id,
        &human_commit,
        &agent_commit,
        Some(&expected_human_record_id),
    )
    .await;
    let (runtime_created_at, runtime_session_id): (OffsetDateTime, Uuid) = sqlx::query_as(
        "SELECT session.created_at, session.id
           FROM runtime_authority_sessions AS session
          WHERE session.run_id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("restart runtime authority session");
    assert!(runtime_created_at > restart_boundary);
    assert!(!previous_runtime_session_ids.contains(&runtime_session_id));
    assert_ne!(runtime_session_id, Uuid::nil());
    for (path, expected) in &initial_record_blobs {
        assert_eq!(
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await,
            expected.as_slice(),
            "pre-restart record blob changed: {path}"
        );
    }
    let run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("restart session run request count");
    assert_eq!(
        run_request_count, 4,
        "restart must not recursively schedule runs"
    );
    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("restart session Git receive count");
    assert_eq!(
        receive_count,
        initial_receive_count + 2,
        "restart must add exactly one human and one assistant receive"
    );
    let (current_installation_id, current_generation_id, current_release_id): (Uuid, Uuid, Uuid) =
        sqlx::query_as(
            "SELECT installation.id, installation.current_generation_id, generation.release_id
           FROM ui_installations installation
           JOIN ui_installation_generations generation
             ON generation.id = installation.current_generation_id
          WHERE installation.id = $1
            AND installation.project_id = $2
            AND installation.repository_id = $3
            AND installation.lifecycle = 'enabled'
            AND generation.release_id = $4",
        )
        .bind(installation_id)
        .bind(project.as_uuid())
        .bind(repository_id)
        .bind(release_id)
        .fetch_one(pool)
        .await
        .expect("restart installed UI persistence");
    assert_eq!(current_installation_id, installation_id);
    assert_eq!(current_generation_id, generation_id);
    assert_eq!(current_release_id, release_id);
    let current_revision_id: Uuid =
        sqlx::query_scalar("SELECT active_revision_id FROM agent_instances WHERE id = $1")
            .bind(instance_id)
            .fetch_one(pool)
            .await
            .expect("restart active instance revision");
    assert_eq!(current_revision_id, revision_id);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=restart-canonical-validation-passed turns=3");
    if concurrent_e2e_enabled() {
        let broker = exercise_browser_concurrency(
            pool,
            database_url,
            running,
            root,
            broker,
            project,
            actor_id,
            release_agent_id,
            repository_id,
            instance_id,
            revision_id,
            attachment_id,
            installation_id,
            generation_id,
            session_id,
        )
        .await;
        if fork_e2e_enabled() {
            let source_head =
                git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
            let reachable_objects = git_output_bare(
                root,
                repository_id,
                &["rev-list", "--objects", &source_head],
            )
            .await
            .lines()
            .filter_map(|line| line.split_whitespace().next().map(str::to_owned))
            .collect::<BTreeSet<_>>();
            assert!(!reachable_objects.is_empty());
            let record_paths = git_output_bare(
                root,
                repository_id,
                &["ls-tree", "-r", "--name-only", &source_head],
            )
            .await
            .lines()
            .filter(|path| path.starts_with(".heph/session/v1/records/"))
            .map(str::to_owned)
            .collect::<Vec<_>>();
            let mut record_blobs = BTreeMap::new();
            for path in record_paths {
                record_blobs.insert(
                    path.clone(),
                    git_output_bare_bytes(
                        root,
                        repository_id,
                        &["show", &format!("{source_head}:{path}")],
                    )
                    .await,
                );
            }
            assert_eq!(record_blobs.len(), 10);
            let (model_rule_id, model_binding_id): (Uuid, Uuid) = sqlx::query_as(
                "SELECT rule.id, rule.binding_id
                   FROM brokered_secret_rules AS rule
                   JOIN agent_secret_bindings AS binding
                     ON binding.id = rule.binding_id
                    AND binding.instance_revision_id = rule.instance_revision_id
                  WHERE rule.instance_revision_id = $1
                    AND binding.slot_key = 'model'
                    AND binding.status = 'active'
                    AND rule.destination_origin = 'https://api.model.example'",
            )
            .bind(revision_id)
            .fetch_one(pool)
            .await
            .expect("source session model rule and binding");
            let source = fork::SourceSessionState {
                repository_id,
                session_id,
                release_id,
                release_agent_id,
                model_rule_id,
                instance_id,
                revision_id,
                attachment_id,
                installation_id,
                generation_id,
                model_binding_id,
                head: source_head,
                reachable_objects,
                record_blobs,
                accepted_receive_count: accepted_receive_count(pool, repository_id).await,
            };
            exercise_browser_fork(
                pool,
                database_url,
                running,
                root,
                source_root,
                project,
                organization,
                identity,
                git_token,
                rpc_token,
                broker,
                source,
            )
            .await;
            return;
        }
        let final_requests = broker.assert_observed().await;
        assert_eq!(
            final_requests.len(),
            5,
            "concurrency flow must make five model turns"
        );
    } else {
        let final_requests = broker.assert_observed().await;
        assert_eq!(
            final_requests.len(),
            3,
            "restart flow must make three model turns"
        );
    }
}

/// Publishes and exercises the forked repository while retaining the source
/// broker observer.  The caller owns the restart boundary and supplies the
/// same production identity/token factories used by the source phase.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)] // Fork host assertions intentionally cover Git, browser, and runtime provenance together.
async fn exercise_browser_fork(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    source_root: &Path,
    project: ProjectId,
    organization: OrganizationId,
    identity: &AuthenticatedIdentity,
    git_token: &str,
    rpc_token: &(dyn Fn(&str) -> String + Send + Sync),
    broker: SessionBrokerFixture,
    source: fork::SourceSessionState,
) {
    let source_before = source.clone();
    let target = fork::exercise(
        pool,
        running,
        root,
        source_root,
        project,
        source,
        git_token,
        rpc_token,
    )
    .await;
    assert_eq!(target.source_repository_id, source_before.repository_id);
    assert_eq!(target.source_head, source_before.head);
    assert_eq!(
        target.source_model_binding_id,
        source_before.model_binding_id
    );
    assert_eq!(
        target.source_accepted_receive_count,
        source_before.accepted_receive_count
    );
    assert!(
        target.checkout.is_dir(),
        "fork checkout must remain materialized"
    );
    assert!(!target.manifest_commit.is_empty());
    let target_rule_id = Uuid::new_v4();
    let provisioned = fork::provision_target(
        pool,
        running,
        project,
        organization,
        identity,
        rpc_token,
        &target,
        target.source_release_id,
        target.source_release_agent_id,
        target_rule_id,
    )
    .await;
    assert_ne!(provisioned.revision_id, source_before.revision_id);
    assert_ne!(
        provisioned.capability_revision_id,
        source_before.revision_id
    );
    assert_ne!(provisioned.bound_revision_id, source_before.revision_id);
    assert_ne!(provisioned.binding_id, source_before.model_binding_id);
    let binding_revision_id: Uuid =
        sqlx::query_scalar("SELECT instance_revision_id FROM agent_secret_bindings WHERE id = $1")
            .bind(provisioned.binding_id)
            .fetch_one(pool)
            .await
            .expect("fork target fresh model binding");
    assert_eq!(binding_revision_id, provisioned.bound_revision_id);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=fork-started");
    run_session_chat_browser(
        database_url,
        running,
        project,
        target.source_release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Fork {
            repository_id: target.repository_id,
            installation_id: provisioned.installation_id,
            generation_id: provisioned.generation_id,
            actor_id: identity.user_id.as_uuid(),
            initial_transcript_count: 10,
            initial_agent_count: 5,
        },
    )
    .await;
    broker.wait_for_observed(6).await;
    let requests = broker.observed_snapshot();
    assert_eq!(requests.len(), 6, "fork flow must make six model turns");
    let target_request = &requests[5];
    assert_ne!(target_request.session_id, source_before.session_id);
    assert_eq!(target_request.session_id, target.session_id);
    assert_eq!(target_request.messages.len(), 11);
    for (index, message) in target_request.messages.iter().enumerate() {
        assert_eq!(
            message.role,
            if index % 2 == 0 { "user" } else { "assistant" }
        );
    }
    assert_eq!(
        target_request
            .messages
            .last()
            .map(|message| message.record_id),
        Some(target_request.record_id)
    );

    let target_head = git_output_bare(
        root,
        target.repository_id,
        &["rev-parse", "refs/heads/main"],
    )
    .await;
    let source_commit_count = git_output_bare(
        root,
        source_before.repository_id,
        &["rev-list", &source_before.head],
    )
    .await
    .lines()
    .count();
    let target_commit_count =
        git_output_bare(root, target.repository_id, &["rev-list", &target_head])
            .await
            .lines()
            .count();
    assert_eq!(
        target_commit_count,
        source_commit_count + 3,
        "target history must contain the source, manifest, human, and assistant commits only"
    );
    let target_paths = git_output_bare(
        root,
        target.repository_id,
        &["ls-tree", "-r", "--name-only", &target_head],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let human_paths = target_paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .cloned()
        .collect::<Vec<_>>();
    let agent_paths = target_paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        6,
        "fork target must retain six human records"
    );
    assert_eq!(
        agent_paths.len(),
        6,
        "fork target must retain six assistant records"
    );
    for (path, expected) in &source_before.record_blobs {
        assert_eq!(
            git_output_bare_bytes(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
            expected.as_slice(),
            "fork target source record changed after target turn: {path}"
        );
    }
    let target_manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            target.repository_id,
            &["show", &format!("{target_head}:{}", target.manifest_path)],
        )
        .await,
    )
    .expect("fork target manifest JSON");
    assert_eq!(
        target_manifest["data"]["session_id"],
        target.session_id.to_string()
    );
    assert_eq!(
        target_manifest["data"]["forked_from_session_id"],
        source_before.session_id.to_string()
    );

    let mut human_record_paths = HashMap::new();
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
        )
        .expect("fork target human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("fork target human record ID"),
        )
        .expect("fork target human record UUID");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_record_paths = HashMap::new();
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(
                root,
                target.repository_id,
                &["show", &format!("{target_head}:{path}")],
            )
            .await,
        )
        .expect("fork target assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("fork target assistant record ID"),
        )
        .expect("fork target assistant record UUID");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, (path.clone(), record));
    }
    let human_path = human_record_paths
        .get(&target_request.record_id)
        .expect("fork target request human record");
    let human_commit = canonical_record_commit(root, target.repository_id, human_path).await;
    let agent_entry = agent_record_paths
        .values()
        .find(|(_, record)| record["in_reply_to"] == target_request.record_id.to_string())
        .expect("fork target assistant response");
    let agent_commit = canonical_record_commit(root, target.repository_id, &agent_entry.0).await;
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", &format!("{human_commit}^")],
        )
        .await,
        target.manifest_commit,
        "target human commit must directly follow the fork manifest"
    );
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", &format!("{agent_commit}^")],
        )
        .await,
        human_commit,
        "fork target assistant commit must directly follow its human commit"
    );
    let run_id = accepted_normal_run_id(
        pool,
        target.repository_id,
        provisioned.instance_id,
        provisioned.attachment_id,
        identity.user_id.as_uuid(),
        &human_commit,
    )
    .await;
    wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
    let expected_human_record_id = target_request.record_id.to_string();
    assert_runtime_git_turn_at_commit(
        pool,
        root,
        target.repository_id,
        provisioned.instance_id,
        provisioned.attachment_id,
        identity.user_id.as_uuid(),
        run_id,
        &human_commit,
        &agent_commit,
        Some(&expected_human_record_id),
    )
    .await;
    let (runtime_revision_id, authority_revision_id, runtime_instance_id, runtime_attachment_id): (
        Uuid,
        Uuid,
        Uuid,
        Uuid,
    ) = sqlx::query_as(
        "SELECT run.instance_revision_id, session.instance_revision_id,
                    session.instance_id, session.attachment_id
               FROM runs AS run
               JOIN runtime_authority_sessions AS session ON session.run_id = run.id
              WHERE run.id = $1",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("fork target runtime authority provenance");
    assert_eq!(runtime_revision_id, provisioned.bound_revision_id);
    assert_eq!(authority_revision_id, provisioned.bound_revision_id);
    assert_eq!(runtime_instance_id, provisioned.instance_id);
    assert_eq!(runtime_attachment_id, provisioned.attachment_id);
    assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
    let target_receive_count = accepted_receive_count(pool, target.repository_id).await;
    assert_eq!(
        target_receive_count,
        target.initial_accepted_receive_count + 2,
        "target turn must add exactly one human and one runtime receive"
    );
    let target_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM run_requests
          WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(provisioned.instance_id)
    .bind(target.repository_id)
    .fetch_one(pool)
    .await
    .expect("fork target run request count");
    assert_eq!(
        target_run_request_count, 1,
        "fork target must schedule only its one new model turn"
    );
    assert_eq!(
        git_output_bare(
            root,
            target.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        target_head
    );
    assert_eq!(
        target_head, agent_commit,
        "target main must finish at the assistant commit"
    );
    assert_eq!(
        git_output_bare(
            root,
            source_before.repository_id,
            &["rev-parse", "refs/heads/main"]
        )
        .await,
        source_before.head
    );
    assert_eq!(
        accepted_receive_count(pool, source_before.repository_id).await,
        source_before.accepted_receive_count
    );
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=fork-canonical-validation-passed target_turns=6");
    let _ = broker.assert_observed().await;
}

// Keep the concurrency assertions together so each production boundary is checked once.
#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
async fn exercise_browser_concurrency(
    pool: &PgPool,
    database_url: &str,
    running: &RunningHephaestus,
    root: &Path,
    broker: SessionBrokerFixture,
    project: ProjectId,
    actor_id: Uuid,
    release_agent_id: Uuid,
    repository_id: Uuid,
    instance_id: Uuid,
    revision_id: Uuid,
    attachment_id: Uuid,
    installation_id: Uuid,
    generation_id: Uuid,
    session_id: Uuid,
) -> SessionBrokerFixture {
    let before_head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    let before_record_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", before_head.trim()],
    )
    .await
    .lines()
    .filter(|path| path.starts_with(".heph/session/v1/records/"))
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let mut before_record_blobs = HashMap::with_capacity(before_record_paths.len());
    for path in before_record_paths {
        let content = git_output_bare_bytes(
            root,
            repository_id,
            &["show", &format!("{before_head}:{path}")],
        )
        .await;
        before_record_blobs.insert(path, content);
    }
    let before_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency accepted receive baseline");
    let before_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency run request baseline");

    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=concurrency-started");
    run_session_chat_browser(
        database_url,
        running,
        project,
        release_agent_id,
        Uuid::nil(),
        SessionChatBrowserMode::Concurrent {
            repository_id,
            installation_id,
            generation_id,
            actor_id,
            initial_transcript_count: 6,
            initial_agent_count: 3,
        },
    )
    .await;
    broker.wait_for_observed(5).await;
    let requests = broker.observed_snapshot();
    assert_eq!(
        requests.len(),
        5,
        "full session-chat journey must make five model turns"
    );
    let concurrent_requests = &requests[3..];
    assert_eq!(concurrent_requests.len(), 2);
    let mut concurrent_record_ids = HashSet::new();
    for request in concurrent_requests {
        assert_eq!(request.session_id, session_id);
        assert!(concurrent_record_ids.insert(request.record_id));
        let message = request
            .messages
            .last()
            .expect("concurrent model request human message");
        assert_eq!(message.record_id, request.record_id);
        assert_eq!(message.role, "user");
    }

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"]).await;
    assert_ne!(
        head, before_head,
        "concurrency must persist a new canonical head"
    );
    let repository_path = root
        .join("repositories")
        .join(format!("{repository_id}.git"));
    let ancestry = Command::new("git")
        .arg(format!("--git-dir={}", repository_path.display()))
        .args(["merge-base", "--is-ancestor", &before_head, &head])
        .status()
        .await
        .expect("check concurrency Git ancestry");
    assert!(
        ancestry.success(),
        "concurrency must preserve the pre-race history"
    );
    for (path, expected) in &before_record_blobs {
        assert_eq!(
            git_output_bare_bytes(root, repository_id, &["show", &format!("{head}:{path}")]).await,
            expected.as_slice(),
            "pre-concurrency record blob changed: {path}"
        );
    }

    let paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", head.trim()],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let human_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .cloned()
        .collect::<Vec<_>>();
    let agent_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        5,
        "concurrency must retain five human records"
    );
    assert_eq!(
        agent_paths.len(),
        5,
        "concurrency must retain five assistant records"
    );

    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in &human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("concurrency human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("concurrency human record ID"),
        )
        .expect("concurrency human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, path.clone());
    }
    let mut agent_records = HashMap::with_capacity(agent_paths.len());
    for path in &agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("concurrency assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("concurrency assistant record ID"),
        )
        .expect("concurrency assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_records.insert(record_id, (path.clone(), record));
    }

    let mut run_ids = Vec::with_capacity(concurrent_requests.len());
    for request in concurrent_requests {
        let human_path = human_record_paths
            .get(&request.record_id)
            .expect("concurrent model request human record");
        let human_commit = canonical_record_commit(root, repository_id, human_path).await;
        let ancestor = Command::new("git")
            .arg(format!("--git-dir={}", repository_path.display()))
            .args(["merge-base", "--is-ancestor", &before_head, &human_commit])
            .status()
            .await
            .expect("check concurrent human ancestry");
        assert!(
            ancestor.success(),
            "concurrent human commit must descend from the race head"
        );
        let agent_entry = agent_records
            .values()
            .find(|entry| entry.1["in_reply_to"] == request.record_id.to_string())
            .expect("concurrent assistant response");
        let agent_path = &agent_entry.0;
        let agent_record = &agent_entry.1;
        assert_eq!(agent_record["in_reply_to"], request.record_id.to_string());
        let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
        assert_eq!(
            git_output_bare(
                root,
                repository_id,
                &["rev-parse", &format!("{agent_commit}^")]
            )
            .await,
            human_commit,
            "concurrent assistant commit must directly parent its human commit"
        );
        let expected_human_record_id = request.record_id.to_string();
        let run_id = accepted_normal_run_id(
            pool,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            &human_commit,
        )
        .await;
        let revision_matches: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM run_requests
              WHERE run_id = $1 AND instance_revision_id = $2 AND attachment_id = $3",
        )
        .bind(run_id)
        .bind(revision_id)
        .bind(attachment_id)
        .fetch_one(pool)
        .await
        .expect("concurrency immutable revision provenance");
        assert_eq!(revision_matches, 1);
        wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
        assert_eq!(accepted_runtime_receive_count(pool, run_id).await, 1);
        assert_runtime_git_turn_at_commit(
            pool,
            root,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            run_id,
            &human_commit,
            &agent_commit,
            Some(&expected_human_record_id),
        )
        .await;
        run_ids.push(run_id);
    }

    let after_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives WHERE repository_id = $1 AND status = 'accepted'",
    )
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency accepted receive count");
    assert_eq!(
        after_receive_count,
        before_receive_count + 4,
        "concurrency must persist two human and two runtime accepted receives"
    );
    let after_run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM run_requests WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("concurrency run request count");
    assert_eq!(
        after_run_request_count,
        before_run_request_count + 2,
        "stale concurrency attempt must not create a run request"
    );
    let runtime_receive_count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM git_receives AS receive
          JOIN runtime_authority_sessions AS session
            ON session.id = receive.runtime_session_id
         WHERE session.run_id = ANY($1) AND receive.status = 'accepted'",
    )
    .bind(&run_ids)
    .fetch_one(pool)
    .await
    .expect("concurrency runtime receive count");
    assert_eq!(runtime_receive_count, 2);
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=concurrency-canonical-validation-passed turns=5");
    broker
}

async fn canonical_record_commit(root: &Path, repository_id: Uuid, record_path: &str) -> String {
    let commits = git_output_bare(
        root,
        repository_id,
        &[
            "log",
            "--all",
            "--diff-filter=A",
            "--format=%H",
            "--",
            record_path,
        ],
    )
    .await;
    let commits = commits.lines().collect::<Vec<_>>();
    assert_eq!(
        commits.len(),
        1,
        "canonical record must be added by exactly one commit: {record_path}"
    );
    commits[0].to_owned()
}

async fn accepted_normal_run_id(
    pool: &PgPool,
    repository_id: Uuid,
    instance_id: Uuid,
    attachment_id: Uuid,
    actor_id: Uuid,
    commit: &str,
) -> Uuid {
    sqlx::query_scalar(
        "SELECT request.run_id
           FROM run_requests AS request
           JOIN git_ref_updates AS update ON update.receive_id = request.receive_id
           JOIN git_receives AS receive ON receive.id = update.receive_id
          WHERE request.repository_id = $1
            AND request.instance_id = $2
            AND request.commit_sha = $3
            AND request.git_ref = 'refs/heads/main'
            AND request.request_kind = 'instance_normal'
            AND request.attachment_id = $4
            AND receive.repository_id = $1
            AND receive.actor_id = $5
            AND receive.status = 'accepted'
            AND receive.runtime_session_id IS NULL
            AND receive.runtime_attachment_id IS NULL
            AND update.git_ref = 'refs/heads/main'
            AND update.new_commit = $3
          LIMIT 1",
    )
    .bind(repository_id)
    .bind(instance_id)
    .bind(commit)
    .bind(attachment_id)
    .bind(actor_id)
    .fetch_one(pool)
    .await
    .expect("accepted browser session run request")
}

async fn accepted_runtime_receive_count(pool: &PgPool, run_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT count(*)
           FROM git_receives AS receive
           JOIN runtime_authority_sessions AS session
             ON session.id = receive.runtime_session_id
          WHERE session.run_id = $1
            AND receive.status = 'accepted'",
    )
    .bind(run_id)
    .fetch_one(pool)
    .await
    .expect("runtime receive provenance count")
}

#[allow(clippy::cognitive_complexity, clippy::too_many_lines)] // This is one reviewable assertion over the browser's persisted Git session.
async fn assert_browser_session(
    pool: &PgPool,
    root: &Path,
    project: ProjectId,
    actor_id: Uuid,
    release_agent_id: Uuid,
    existing_repository_ids: &HashSet<Uuid>,
    requests: Vec<ObservedModelRequest>,
) {
    assert_eq!(
        requests.len(),
        2,
        "browser session must make two model turns"
    );
    let candidates: Vec<BrowserSessionObjects> = sqlx::query_as(
        "SELECT repository.id AS repository_id,
                instance.id AS instance_id,
                revision.id AS revision_id,
                attachment.id AS attachment_id
           FROM repositories repository
           JOIN agent_attachments attachment
             ON attachment.repository_id = repository.id
            AND attachment.project_id = repository.project_id
           JOIN agent_instances instance
             ON instance.id = attachment.instance_id
            AND instance.project_id = repository.project_id
           JOIN agent_instance_revisions revision
             ON revision.id = instance.active_revision_id
            AND revision.instance_id = instance.id
            AND revision.release_agent_id = $2
          WHERE repository.project_id = $1
            AND attachment.ref_selector = 'refs/heads/main'
            AND attachment.removed_at IS NULL",
    )
    .bind(project.as_uuid())
    .bind(release_agent_id)
    .fetch_all(pool)
    .await
    .expect("browser session repository and attachment");
    let new_candidates: Vec<_> = candidates
        .into_iter()
        .filter(|candidate| !existing_repository_ids.contains(&candidate.repository_id))
        .collect();
    assert_eq!(
        new_candidates.len(),
        1,
        "browser session must create one project repository attached to the selected release"
    );
    let BrowserSessionObjects {
        repository_id,
        instance_id,
        revision_id,
        attachment_id,
    } = new_candidates
        .into_iter()
        .next()
        .expect("new browser session");
    assert!(!instance_id.is_nil());
    assert!(!revision_id.is_nil());
    assert!(!attachment_id.is_nil());

    let head = git_output_bare(root, repository_id, &["rev-parse", "refs/heads/main"])
        .await
        .trim()
        .to_owned();
    let paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", head.trim()],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    let manifest_path = ".heph/session/v1/manifest.json";
    assert!(paths.iter().any(|path| path == manifest_path));
    let manifest: JsonValue = serde_json::from_str(
        &git_output_bare(
            root,
            repository_id,
            &["show", &format!("{head}:{manifest_path}")],
        )
        .await,
    )
    .expect("browser session manifest JSON");
    let session_id = Uuid::parse_str(
        manifest["data"]["session_id"]
            .as_str()
            .expect("browser session manifest session ID"),
    )
    .expect("browser session manifest session UUID");

    let human_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/human/"))
        .collect::<Vec<_>>();
    let agent_paths = paths
        .iter()
        .filter(|path| path.starts_with(".heph/session/v1/records/agent/"))
        .collect::<Vec<_>>();
    assert_eq!(
        human_paths.len(),
        2,
        "browser session must publish two human records"
    );
    assert_eq!(
        agent_paths.len(),
        2,
        "browser session must publish two assistant records"
    );
    let mut human_records = Vec::with_capacity(human_paths.len());
    let mut human_record_paths = HashMap::with_capacity(human_paths.len());
    for path in human_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("browser human record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("browser human record ID"),
        )
        .expect("browser human record UUID");
        assert_eq!(record["kind"], "user_message");
        human_record_paths.insert(record_id, (*path).clone());
        human_records.push((record_id, record));
    }
    let mut agent_records = Vec::with_capacity(agent_paths.len());
    let mut agent_record_paths = HashMap::with_capacity(agent_paths.len());
    for path in agent_paths {
        let record: JsonValue = serde_json::from_str(
            &git_output_bare(root, repository_id, &["show", &format!("{head}:{path}")]).await,
        )
        .expect("browser assistant record JSON");
        let record_id = Uuid::parse_str(
            record["record_id"]
                .as_str()
                .expect("browser assistant record ID"),
        )
        .expect("browser assistant record UUID");
        assert_eq!(record["kind"], "assistant_message");
        assert_eq!(record["content"]["text"], MODEL_RESPONSE_TEXT);
        agent_record_paths.insert(record_id, (*path).clone());
        agent_records.push((record_id, record));
    }

    let first = &requests[0];
    let second = &requests[1];
    assert_eq!(first.session_id, session_id);
    assert_eq!(second.session_id, session_id);
    assert_ne!(first.record_id, second.record_id);
    assert!(
        human_records
            .iter()
            .any(|(record_id, _)| *record_id == first.record_id)
    );
    assert!(
        human_records
            .iter()
            .any(|(record_id, _)| *record_id == second.record_id)
    );
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.messages[0].role, "user");
    assert_eq!(second.messages.len(), 3);
    assert_eq!(second.messages[0].role, "user");
    assert_eq!(second.messages[1].role, "assistant");
    assert_eq!(second.messages[2].role, "user");
    assert_eq!(first.messages[0].record_id, first.record_id);
    assert_eq!(second.messages[0].record_id, first.record_id);
    assert_eq!(second.messages[2].record_id, second.record_id);
    let first_agent = agent_records
        .iter()
        .find(|(_, record)| record["in_reply_to"] == first.record_id.to_string())
        .expect("first assistant response");
    let second_agent = agent_records
        .iter()
        .find(|(_, record)| record["in_reply_to"] == second.record_id.to_string())
        .expect("second assistant response");
    assert_eq!(second.messages[1].record_id, first_agent.0);
    assert_ne!(first_agent.0, second_agent.0);

    let first_human_path = human_record_paths
        .get(&first.record_id)
        .expect("first model request human record path");
    let first_human_commit = canonical_record_commit(root, repository_id, first_human_path).await;
    let initialization_commit = git_output_bare(
        root,
        repository_id,
        &["rev-parse", &format!("{first_human_commit}^")],
    )
    .await;
    let initialization_parent_line = git_output_bare(
        root,
        repository_id,
        &["rev-list", "--parents", "-n", "1", &initialization_commit],
    )
    .await;
    let initialization_parents = initialization_parent_line
        .split_whitespace()
        .collect::<Vec<_>>();
    assert_eq!(
        initialization_parents,
        [initialization_commit.as_str()],
        "session initialization must be the root commit"
    );
    let initialization_paths = git_output_bare(
        root,
        repository_id,
        &["ls-tree", "-r", "--name-only", &initialization_commit],
    )
    .await
    .lines()
    .map(str::to_owned)
    .collect::<Vec<_>>();
    assert_eq!(
        initialization_paths,
        [
            ".heph/session/v1/manifest.json".to_owned(),
            ".heph/session/v1/participants/agent%3Areference-chat.json".to_owned(),
            ".heph/session/v1/participants/release%3Areference-chat.json".to_owned(),
            format!(".heph/session/v1/participants/user%3A{actor_id}.json"),
        ],
        "session initialization must publish only its manifest and participants"
    );
    let initialization_run_id = accepted_normal_run_id(
        pool,
        repository_id,
        instance_id,
        attachment_id,
        actor_id,
        &initialization_commit,
    )
    .await;
    wait_for_run_succeeded(pool, initialization_run_id, Duration::from_secs(120)).await;
    let initialization_runtime_receives =
        accepted_runtime_receive_count(pool, initialization_run_id).await;
    assert_eq!(
        initialization_runtime_receives, 0,
        "idle initialization must not publish a runtime-authenticated receive"
    );

    for request in [first, second] {
        let human_path = human_record_paths
            .get(&request.record_id)
            .expect("model request human record path");
        let human_commit = canonical_record_commit(root, repository_id, human_path).await;
        let agent = agent_records
            .iter()
            .find(|(_, record)| record["in_reply_to"] == request.record_id.to_string())
            .expect("assistant response for each model request");
        let run_id = accepted_normal_run_id(
            pool,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            &human_commit,
        )
        .await;
        wait_for_run_succeeded(pool, run_id, Duration::from_secs(120)).await;
        assert_eq!(
            accepted_runtime_receive_count(pool, run_id).await,
            1,
            "each human turn must have one accepted runtime-authenticated receive"
        );
        let agent_path = agent_record_paths
            .get(&agent.0)
            .expect("assistant record path");
        let agent_commit = canonical_record_commit(root, repository_id, agent_path).await;
        assert_eq!(
            git_output_bare(
                root,
                repository_id,
                &["rev-parse", &format!("{agent_commit}^")],
            )
            .await,
            human_commit,
            "assistant commit must be directly based on its human commit"
        );
        let expected_human_record_id = request.record_id.to_string();
        assert_runtime_git_turn_at_commit(
            pool,
            root,
            repository_id,
            instance_id,
            attachment_id,
            actor_id,
            run_id,
            &human_commit,
            &agent_commit,
            Some(&expected_human_record_id),
        )
        .await;
    }

    // The new-session browser flow asserts an empty transcript immediately
    // after opening the UI; its initialization creates one idle run, and its
    // two explicit Send steps create the only model-bearing runs.
    let run_request_count: i64 = sqlx::query_scalar(
        "SELECT count(*)
           FROM run_requests
          WHERE instance_id = $1 AND repository_id = $2",
    )
    .bind(instance_id)
    .bind(repository_id)
    .fetch_one(pool)
    .await
    .expect("browser session run request count");
    assert_eq!(
        run_request_count, 3,
        "browser initialization must schedule one idle run; two human runs must not recurse"
    );
    let receive_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM git_receives WHERE repository_id = $1")
            .bind(repository_id)
            .fetch_one(pool)
            .await
            .expect("browser session Git receive count");
    assert!(
        receive_count >= 4,
        "browser session must persist both turns and reconnect Git receives"
    );
    if let Ok(diagnostics_dir) = std::env::var("HEPHAESTUS_COOKING_DIAGNOSTICS_DIR") {
        let report = serde_json::json!({
            "mode": "session_chat_new",
            "repository_id": repository_id,
            "instance_id": instance_id,
            "revision_id": revision_id,
            "attachment_id": attachment_id,
            "session_id": session_id,
            "model_requests": requests.iter().map(|request| serde_json::json!({
                "session_id": request.session_id,
                "record_id": request.record_id,
                "message_ids": request.messages.iter().map(|message| message.record_id).collect::<Vec<_>>(),
                "message_roles": request.messages.iter().map(|message| message.role.as_str()).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "human_record_ids": human_records.iter().map(|(record_id, _)| record_id).collect::<Vec<_>>(),
            "assistant_record_ids": agent_records.iter().map(|(record_id, _)| record_id).collect::<Vec<_>>(),
            "git_receive_count": receive_count,
        });
        tokio::fs::create_dir_all(&diagnostics_dir)
            .await
            .expect("browser session diagnostics directory");
        tokio::fs::write(
            Path::new(&diagnostics_dir).join("session-chat-browser-report.json"),
            serde_json::to_vec_pretty(&report).expect("browser session diagnostics JSON"),
        )
        .await
        .expect("browser session diagnostics report");
    }
    eprintln!("HEPH_SESSION_CHAT_BROWSER stage=canonical-validation-passed turns=2");
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
    human_record_id: &str,
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
        .arg(human_record_id)
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

async fn seed_model_import(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
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
    import_id.as_uuid()
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
    seed_model_secret_with_rule(
        pool,
        organization,
        project,
        identity,
        instance,
        revision,
        attachment,
        MODEL_RULE,
    )
    .await
    .0
}

#[allow(clippy::too_many_arguments)] // Secret binding IDs and target rule are all independent acceptance inputs.
async fn seed_model_secret_with_rule(
    pool: &PgPool,
    organization: OrganizationId,
    project: ProjectId,
    identity: &AuthenticatedIdentity,
    instance: Uuid,
    revision: Uuid,
    attachment: Uuid,
    model_rule: Uuid,
) -> (Uuid, Uuid) {
    let import_id =
        SecretImportId::from_uuid(seed_model_import(pool, organization, project, identity).await);
    let service = SecretService::new(
        pool.clone(),
        EncryptedStore::new(
            LocalKeyProvider::new("golden/v1", [("golden/v1", [17_u8; 32])])
                .expect("session secret key"),
        ),
        Arc::new(super::PostgresMelangeAuthorizer),
    );
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
                command_key: secret_command_key("session-chat-rule", model_rule),
                rule_id: model_rule,
                binding_id,
                destination: String::from("https://api.model.example"),
                header: String::from("authorization"),
                header_prefix: Some(String::from("Bearer ")),
            },
        )
        .await
        .expect("declare session model rule");
    (bound_revision.as_uuid(), binding_id.as_uuid())
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

async fn git_output_bare_bytes(root: &Path, repository: Uuid, arguments: &[&str]) -> Vec<u8> {
    let bare = root.join("repositories").join(format!("{repository}.git"));
    let output = Command::new("git")
        .arg(format!("--git-dir={}", bare.display()))
        .args(arguments)
        .output()
        .await
        .expect("run session bare Git byte output command");
    assert!(
        output.status.success(),
        "bare Git byte output failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
