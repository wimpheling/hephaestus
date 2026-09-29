// Secret broker test scenarios.
use super::support::*;

#[test]
fn pinned_transport_rejects_private_and_unpinned_addresses() {
    for addresses in [
        Vec::new(),
        vec!["127.0.0.1".parse().expect("address")],
        vec!["169.254.169.254".parse().expect("address")],
        vec!["::1".parse().expect("address")],
        vec!["fc00::1".parse().expect("address")],
    ] {
        assert!(matches!(
            ReqwestPinnedHttpsTransport::new("api.example.test", &addresses),
            Err(BrokerAdapterError::Rejected)
        ));
    }
}
#[tokio::test]
async fn loopback_adapter_rejects_dns_and_body_confusion_before_connecting() {
    for destination in [
        "::1",
        "[::1]",
        "169.254.169.254",
        "metadata.google.internal",
        "localhost.local",
        "-api.example.test",
        "api..example.test",
    ] {
        assert!(matches!(
            LoopbackCompletionAdapter::new(destination, "127.0.0.1:1".parse().expect("address"), 1),
            Err(BrokerAdapterError::Rejected)
        ));
    }
    let adapter = LoopbackCompletionAdapter::new(
        "api.example.test",
        "127.0.0.1:1".parse().expect("address"),
        1,
    )
    .expect("adapter");
    for destination in ["alternate.example.test", "api.example.test."] {
        let result = adapter
            .invoke(
                &SecretValue::new("bounded-credential").expect("credential"),
                destination,
                "complete",
                b"{}",
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    }
    let oversized = vec![b'x'; MAX_ADAPTER_BODY_BYTES + 1];
    let result = adapter
        .invoke(
            &SecretValue::new("bounded-credential").expect("credential"),
            "api.example.test",
            "complete",
            &oversized,
        )
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
}
#[tokio::test]
async fn pinned_transport_fails_closed_on_a_real_tls_handshake_failure() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind plaintext TLS-failure fixture");
    let port = listener.local_addr().expect("listener address").port();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept TLS client");
        // A plaintext reply to a TLS ClientHello deterministically causes
        // rustls certificate/handshake verification to fail before HTTP
        // headers or a substituted credential can be transmitted.
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
            .await
            .expect("write plaintext fixture response");
    });
    let transport = ReqwestPinnedHttpsTransport::test_only_local_pin(
        "api.example.test",
        port,
        "127.0.0.1".parse().expect("loopback address"),
    )
    .expect("test-only local pin");
    let result = transport
        .send(UpstreamHttpsRequest {
            method: BrokeredHttpsMethod::Get,
            destination: format!("api.example.test:{port}"),
            path_and_query: String::from("/tls-failure"),
            headers: Vec::new(),
            body: Vec::new(),
        })
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Retryable)));
    server.await.expect("TLS-failure fixture task");
}
