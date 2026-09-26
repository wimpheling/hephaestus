// Reuse the scenario facade imports so each phase preserves the production fixture context.
#[allow(unused_imports)]
use super::*;
pub(crate) fn credential_class(value: &str) -> &'static str {
    match value {
        MODEL_SENTINEL => "model_initial",
        RELAY_SENTINEL => "relay_initial",
        MODEL_ROTATED_SENTINEL => "model_rotated",
        RELAY_ROTATED_SENTINEL => "relay_rotated",
        _ => "unknown",
    }
}

pub(crate) fn observed_credential_class(headers: &str) -> &'static str {
    for line in headers.lines() {
        let Some(value) = line.strip_prefix("authorization: Bearer ") else {
            continue;
        };
        return credential_class(value);
    }
    "unknown"
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn cooking_upstreams_mode(
    rules: Vec<BrokeredSecretRule>,
    crash_mode: bool,
) -> BrokeredTlsUpstream {
    let update_slice = env::var("HEPHAESTUS_COOKING_UPDATE_E2E").as_deref() == Ok("1");
    let mut adapters = std::collections::HashMap::new();
    let mut servers = Vec::new();
    let relay_directory = tempfile::tempdir().expect("external relay data");
    let relay_database = relay_directory.path().join("relay.sqlite3");
    let update_barrier = update_slice.then(|| Arc::new(crate::CookingUpdateBarrier::new()));
    let revocation_barrier = update_slice.then(|| Arc::new(crate::CookingUpdateBarrier::new()));
    for rule in rules {
        let rule_id = rule.id.as_uuid();
        let model = rule_id == MODEL_RULE || rule_id == CRASH_MODEL_RULE;
        let expected_requests = expected_upstream_requests(crash_mode, update_slice, model);
        let host = if model {
            "api.model.example"
        } else {
            "relay.cooking.example"
        };
        let _installed = rustls::crypto::ring::default_provider().install_default();
        let mut parameters = rcgen::CertificateParams::default();
        parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let ca_key = rcgen::KeyPair::generate().expect("CA key");
        let ca = parameters.self_signed(&ca_key).expect("CA");
        let key = rcgen::KeyPair::generate().expect("TLS key");
        let leaf = rcgen::CertificateParams::new(vec![host.to_owned()])
            .expect("TLS name")
            .signed_by(&key, &ca, &ca_key)
            .expect("TLS certificate");
        let tls = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(key.serialize_der().into()),
            )
            .expect("TLS configuration");
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("upstream listener");
        let port = listener.local_addr().expect("upstream address").port();
        let adapter = Arc::new(
            BrokeredHttpsAdapterRegistry::test_only_local_trusted(
                rule,
                port,
                "127.0.0.1".parse().expect("loopback"),
                ca.pem().as_bytes(),
            )
            .expect("pinned TLS adapter"),
        ) as Arc<dyn secret_application::BrokerAdapter>;
        adapters.insert(rule_id, Arc::clone(&adapter));
        let relay_database = relay_database.clone();
        let model_barrier = model.then(|| update_barrier.clone()).flatten();
        let revocation_barrier = model.then(|| revocation_barrier.clone()).flatten();
        servers.push(tokio::spawn(async move {
            let mut requests = 0;
            let calls = Mutex::new(std::collections::BTreeMap::<String, usize>::new());
            for _ in 0..expected_requests {
                let (stream, _) = listener.accept().await.expect("broker TLS request");
                let mut stream = TlsAcceptor::from(Arc::new(tls.clone()))
                    .accept(stream)
                    .await
                    .expect("verified TLS");
                let request = read_http_request(&mut stream).await;
                let separator = request
                    .windows(4)
                    .position(|part| part == b"\r\n\r\n")
                    .expect("HTTP headers");
                let headers =
                    std::str::from_utf8(&request[..separator]).expect("HTTP header encoding");
                assert!(!headers.contains("heph-placeholder:"));
                let body: serde_json::Value = serde_json::from_slice(&request[separator + 4..])
                    .expect("application body");
                let identity = body["idempotency_key"]
                    .as_str()
                    .expect("outbound idempotency key")
                    .to_owned();
                let expected_credential = if !model
                    && matches!(identity.as_str(), "recipe-48" | "recipe-49")
                {
                    RELAY_ROTATED_SENTINEL
                } else if model
                    && update_slice
                    && matches!(identity.as_str(), "recipe-48" | "recipe-49" | "recipe-50")
                {
                    MODEL_ROTATED_SENTINEL
                } else if model {
                    MODEL_SENTINEL
                } else {
                    RELAY_SENTINEL
                };
                let expected_class = credential_class(expected_credential);
                let observed_class = observed_credential_class(headers);
                assert!(
                    observed_class == expected_class,
                    "host substituted credential: identity={identity} expected_class={expected_class} observed_class={observed_class}"
                );
                let call_number = {
                    let mut calls = calls.lock().expect("upstream call ledger");
                    let entry = calls.entry(identity.clone()).or_default();
                    *entry += 1;
                    let call_number = *entry;
                    drop(calls);
                    call_number
                };
                let response = if model {
                    assert!(headers.starts_with("POST /v1/recipes HTTP/1.1"));
                    let user = body["user_id"].as_str().expect("model user");
                    assert!(matches!(user, "alice" | "bob"));
                    let text = body["text"].as_str().expect("model request text");
                    if identity == "recipe-47" {
                        let barrier = model_barrier
                            .as_ref()
                            .expect("update barrier for deferred recipe");
                        barrier.mark_entered();
                        barrier.release.notified().await;
                    }
                    if identity == "recipe-50" {
                        let barrier = revocation_barrier
                            .as_ref()
                            .expect("revocation barrier for active operation");
                        barrier.mark_entered();
                        barrier.release.notified().await;
                    }
                    if user == "alice" && text == "salad" {
                        assert_eq!(
                            body["context"]["summary"],
                            "A simple family recipe",
                            "later Alice request must use persisted SQLite context"
                        );
                    }
                    if !crash_mode
                        && identity == format!("recipe-{MODEL_FAULT_UPDATE}")
                        && call_number == 1
                    {
                        // A valid HTTP response with an invalid application shape
                        // exercises the model-response validation and retry path.
                        serde_json::json!({"fault": "malformed-model-response"})
                    } else {
                        serde_json::json!({
                            "title": format!("Family {text}"),
                            "summary": "A simple family recipe",
                            "ingredients": [text, "tomatoes"],
                            "steps": ["Prepare ingredients", "Serve the family"]
                        })
                    }
                } else {
                    assert!(headers.starts_with("POST /v1/messages HTTP/1.1"));
                    let response = invoke_relay(body, &relay_database, expected_credential).await;
                    if !crash_mode
                        && identity == format!("recipe-{RELAY_FAULT_UPDATE}")
                        && call_number == 1
                    {
                        // invoke_relay has committed its SQLite ledger entry. Closing
                        // before the HTTP response makes the caller observe loss.
                        drop(stream);
                        requests += 1;
                        continue;
                    }
                    response
                };
                let body = serde_json::to_vec(&response).expect("response JSON");
                let header = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                stream
                    .write_all(header.as_bytes())
                    .await
                    .expect("TLS response header");
                stream
                    .write_all(&body)
                    .await
                    .expect("TLS response body");
                requests += 1;
            }
            assert_eq!(
                requests,
                expected_requests,
                "each declared outbound binding receives the planned physical call count"
            );
            let calls = calls.into_inner().expect("upstream call ledger");
            let mut expected = if crash_mode {
                std::collections::BTreeMap::from([
                    (String::from("recipe-51"), 1),
                    (String::from("recipe-52"), 1),
                    (String::from("recipe-53"), if model { 2 } else { 1 }),
                    (String::from("recipe-54"), if model { 1 } else { 2 }),
                    (String::from("recipe-55"), 1),
                ])
            } else {
                std::collections::BTreeMap::from([
                    (String::from("recipe-42"), 1),
                    (String::from("recipe-43"), 1),
                    (String::from("recipe-44"), 1),
                    (String::from("recipe-45"), if model { 2 } else { 1 }),
                    (String::from("recipe-46"), if model { 1 } else { 2 }),
                ])
            };
            if update_slice && !crash_mode {
                expected.insert(String::from("recipe-47"), 1);
                expected.insert(String::from("recipe-48"), 1);
                expected.insert(String::from("recipe-49"), 1);
                if model {
                    expected.insert(String::from("recipe-50"), 1);
                }
            }
            assert_eq!(calls, expected, "bounded outbound calls are recorded per event");
            if !model {
                assert_relay_ledger(&relay_database, update_slice, crash_mode).await;
                if update_slice {
                    assert!(
                        tokio::time::timeout(Duration::from_secs(2), listener.accept())
                            .await
                            .is_err(),
                        "revoked relay operation must not reach the external endpoint"
                    );
                }
            }
        }));
    }
    let observed = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&observed);
    let server = tokio::spawn(async move {
        // Keep the relay directory alive until all upstream requests and the
        // ledger census complete. Dropping this guard also cleans up on a
        // failing upstream assertion or task panic.
        let _relay_directory = relay_directory;
        for task in servers {
            task.await.expect("cooking upstream task");
        }
        done.store(true, Ordering::SeqCst);
    });
    let registry = Arc::new(CookingAdapters::new(adapters.clone()));
    BrokeredTlsUpstream {
        adapter: Arc::clone(&registry) as Arc<dyn secret_application::BrokerAdapter>,
        cooking_registry: Some(registry),
        rule_adapters: Arc::new(adapters),
        observed,
        server,
        update_barrier,
        revocation_barrier,
    }
}
