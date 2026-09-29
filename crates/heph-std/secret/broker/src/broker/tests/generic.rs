// Secret broker test scenarios.
use super::support::*;

#[tokio::test]
async fn generic_https_adapter_substitutes_only_the_bound_placeholder() {
    let transport = RecordingHttpsTransport::default();
    let rule = outbound_rule();
    let placeholder = rule.placeholder();
    let rule_id = rule.id.as_uuid();
    let adapter = BrokeredHttpsAdapter::new(rule, transport).expect("valid adapter");
    let request = BrokeredHttpsRequest {
        rule_id,
        method: BrokeredHttpsMethod::Post,
        path_and_query: String::from("/v1/messages?bounded=true"),
        headers: vec![BrokeredHttpsHeader {
            name: String::from("authorization"),
            value: format!("Bearer {placeholder}"),
        }],
        body: br#"{\"message\":\"hello\"}"#.to_vec(),
    };
    let response = adapter
        .invoke(
            &SecretValue::new("real-secret-sentinel").expect("secret"),
            "api.example.test",
            "https_v1",
            &serde_json::to_vec(&request).expect("request"),
        )
        .await
        .expect("authorized request");
    assert_eq!(response.body, b"ordinary response");
    let headers = adapter.transport.seen.lock().expect("recording lock");
    assert_eq!(headers.len(), 1);
    assert_eq!(headers[0].value, "Bearer real-secret-sentinel");
    drop(headers);
    assert!(
        !response
            .body
            .windows(b"real-secret-sentinel".len())
            .any(|window| window == b"real-secret-sentinel")
    );
}
#[tokio::test]
async fn dynamic_rules_share_only_the_explicit_origin_transport() {
    let transport = RecordingHttpsTransport::default();
    let credential = SecretValue::new("dynamic-secret-sentinel").expect("secret");
    for rule_id in [Uuid::new_v4(), Uuid::new_v4()] {
        let rule = VerifiedBrokeredHttpsRule {
            rule_id,
            destination_origin: String::from("https://api.example.test"),
            header_name: String::from("authorization"),
            header_prefix: Some(String::from("Bearer ")),
        };
        let request = dynamic_request(rule_id, "api.example.test");
        invoke_dynamic_pinned_request(&transport, "api.example.test", &credential, &request, &rule)
            .await
            .expect("authorized dynamic rule");
    }
    assert_eq!(
        transport
            .seen
            .lock()
            .expect("transport lock")
            .last()
            .expect("captured header")
            .value,
        "Bearer dynamic-secret-sentinel"
    );
}
#[tokio::test]
async fn origin_catalog_rejects_unknown_origin_and_legacy_unknown_rule() {
    let configured_rule = outbound_rule();
    let legacy_registry = BrokeredHttpsAdapterRegistry::new(vec![BrokeredHttpsUpstream {
        rule: configured_rule.clone(),
        addresses: vec!["203.0.113.7".parse().expect("test address")],
    }])
    .expect("legacy registry");
    let registry = BrokeredHttpsAdapterRegistry::new_with_origin_catalog(
        vec![BrokeredHttpsUpstream {
            rule: configured_rule.clone(),
            addresses: vec!["203.0.113.7".parse().expect("test address")],
        }],
        vec![BrokeredHttpsOrigin {
            origin: String::from("https://api.example.test"),
            addresses: vec!["203.0.113.8".parse().expect("test address")],
        }],
    )
    .expect("catalog");
    let unknown_rule_id = Uuid::new_v4();
    let request = dynamic_request(unknown_rule_id, "api.example.test");
    let credential = SecretValue::new("secret").expect("secret");
    let unknown_rule = VerifiedBrokeredHttpsRule {
        rule_id: unknown_rule_id,
        destination_origin: String::from("https://api.example.test"),
        header_name: String::from("authorization"),
        header_prefix: Some(String::from("Bearer ")),
    };
    assert!(matches!(
        legacy_registry
            .invoke_verified_https(&credential, &request, &unknown_rule)
            .await,
        Err(BrokerAdapterError::Rejected)
    ));
    let unknown_origin = VerifiedBrokeredHttpsRule {
        destination_origin: String::from("https://other.example.test"),
        ..unknown_rule
    };
    assert!(matches!(
        registry
            .invoke_verified_https(
                &credential,
                &dynamic_request(unknown_origin.rule_id, "other.example.test"),
                &unknown_origin,
            )
            .await,
        Err(BrokerAdapterError::Rejected)
    ));
}
#[cfg(feature = "test-fixtures")]
#[tokio::test]
async fn origin_catalog_routes_distinct_dynamic_rules_to_one_pinned_origin() {
    let _provider = rustls::crypto::ring::default_provider().install_default();
    let mut ca_parameters = rcgen::CertificateParams::default();
    ca_parameters.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_key = rcgen::KeyPair::generate().expect("generate fixture CA key");
    let ca = ca_parameters.self_signed(&ca_key).expect("sign fixture CA");
    let leaf_parameters = rcgen::CertificateParams::new(vec![String::from("api.example.test")])
        .expect("fixture DNS name");
    let leaf_key = rcgen::KeyPair::generate().expect("generate fixture leaf key");
    let leaf = leaf_parameters
        .signed_by(&leaf_key, &ca, &ca_key)
        .expect("sign fixture leaf");
    let tls = Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![rustls::pki_types::CertificateDer::from(leaf.der().to_vec())],
                rustls::pki_types::PrivateKeyDer::Pkcs8(leaf_key.serialize_der().into()),
            )
            .expect("fixture TLS server"),
    );
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind fixture");
    let port = listener.local_addr().expect("fixture address").port();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (stream, _) = listener.accept().await.expect("accept fixture");
            let mut stream = tokio_rustls::TlsAcceptor::from(Arc::clone(&tls))
                .accept(stream)
                .await
                .expect("trusted handshake");
            let mut request = [0_u8; 4096];
            let read = stream.read(&mut request).await.expect("read request");
            assert!(
                std::str::from_utf8(&request[..read])
                    .expect("HTTP request")
                    .contains("authorization: Bearer dynamic-secret-sentinel")
            );
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .await
                .expect("write response");
        }
    });
    let registry = BrokeredHttpsAdapterRegistry::test_only_local_origin_catalog(
        "https://api.example.test",
        port,
        "127.0.0.1".parse().expect("loopback address"),
        ca.pem().as_bytes(),
    )
    .expect("local origin catalog");
    let credential = SecretValue::new("dynamic-secret-sentinel").expect("credential");
    for rule_id in [Uuid::new_v4(), Uuid::new_v4()] {
        let rule = VerifiedBrokeredHttpsRule {
            rule_id,
            destination_origin: String::from("https://api.example.test"),
            header_name: String::from("authorization"),
            header_prefix: Some(String::from("Bearer ")),
        };
        let response = registry
            .invoke_verified_https(
                &credential,
                &dynamic_request(rule_id, "api.example.test"),
                &rule,
            )
            .await
            .expect("dynamic rule should use pinned origin");
        assert_eq!(response.status, BrokerStatus::Succeeded);
        assert_eq!(response.body, b"ok");
    }
    server.await.expect("fixture server");
}
