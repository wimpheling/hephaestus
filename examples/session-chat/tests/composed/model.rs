#![allow(unused_imports)]
use super::{
    HUMAN_RECORD_ID, SESSION_ID, concurrent_e2e_enabled, denial_probe_enabled, fork_e2e_enabled,
    restart_e2e_enabled,
};
use secret_application::BrokerAdapter;
use secret_broker::BrokeredHttpsAdapterRegistry;
use secret_domain::SecretValue;
/// Deterministic CA-pinned HTTPS model fixture for session-chat.
use serde_json::Value as JsonValue;
use std::{
    collections::HashSet,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use tokio_rustls::TlsAcceptor;
use uuid::Uuid;
pub(crate) struct SessionBrokerFixture {
    adapter: Arc<dyn secret_application::BrokerAdapter>,
    observed: Arc<AtomicUsize>,
    requests: Arc<Mutex<Vec<ObservedModelRequest>>>,
    expected_requests: usize,
    server: tokio::task::JoinHandle<()>,
}

impl SessionBrokerFixture {
    pub(crate) fn adapter(&self) -> Arc<dyn secret_application::BrokerAdapter> {
        Arc::clone(&self.adapter)
    }

    pub(crate) async fn assert_observed(self) -> Vec<ObservedModelRequest> {
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

    pub(crate) async fn wait_for_observed(&self, expected: usize) {
        tokio::time::timeout(Duration::from_secs(20), async {
            while self.observed.load(Ordering::SeqCst) < expected {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("session-chat model prefix observation timeout");
    }

    pub(crate) fn observed_snapshot(&self) -> Vec<ObservedModelRequest> {
        self.requests
            .lock()
            .expect("session model request observer lock")
            .clone()
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ObservedModelRequest {
    pub(crate) session_id: Uuid,
    pub(crate) record_id: Uuid,
    pub(crate) messages: Vec<ObservedModelMessage>,
}

#[derive(Clone, Debug)]
pub(crate) struct ObservedModelMessage {
    pub(crate) record_id: Uuid,
    pub(crate) role: String,
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
