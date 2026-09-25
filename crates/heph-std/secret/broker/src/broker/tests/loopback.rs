// Secret broker test scenarios.
use super::support::*;

#[tokio::test]
async fn loopback_adapter_applies_bearer_and_returns_only_sanitized_json() {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fake upstream");
    let address = listener.local_addr().expect("fake upstream address");
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept fake request");
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .await
            .expect("read fake request");
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                  Connection: close\r\n\r\n{\"result\":\"accepted\"}",
            )
            .await
            .expect("write fake response");
        request
    });
    let adapter = LoopbackCompletionAdapter::new("api.example.test", address, 1).expect("adapter");
    let sentinel = b"brokered-credential-sentinel";
    let response = adapter
        .invoke(
            &SecretValue::new(sentinel).expect("credential"),
            "api.example.test",
            "complete",
            br#"{"prompt":"bounded"}"#,
        )
        .await
        .expect("authorized fake operation");
    assert_eq!(response.status, BrokerStatus::Succeeded);
    assert_eq!(response.body, br#"{"result":"accepted"}"#);
    assert!(
        !response
            .body
            .windows(sentinel.len())
            .any(|window| window == sentinel)
    );
    let request = upstream.await.expect("fake upstream task");
    let authorization = [b"Authorization: Bearer ".as_slice(), sentinel.as_slice()].concat();
    assert!(
        request
            .windows(authorization.len())
            .any(|window| window == authorization)
    );
    assert!(
        request
            .windows(b"Host: api.example.test".len())
            .any(|window| window == b"Host: api.example.test")
    );
}
#[tokio::test]
async fn loopback_adapter_rejects_redirects_credential_echo_and_address_injection() {
    assert!(matches!(
        LoopbackCompletionAdapter::new("127.0.0.1", "127.0.0.1:1".parse().expect("address"), 1),
        Err(BrokerAdapterError::Rejected)
    ));
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("fake upstream");
    let address = listener.local_addr().expect("fake upstream address");
    let upstream = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept fake request");
        let mut request = Vec::new();
        stream
            .read_to_end(&mut request)
            .await
            .expect("read fake request");
        stream
            .write_all(
                b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/\r\n\
                  Connection: close\r\n\r\n{\"result\":\"brokered-credential-sentinel\"}",
            )
            .await
            .expect("write fake response");
    });
    let adapter = LoopbackCompletionAdapter::new("api.example.test", address, 1).expect("adapter");
    let result = adapter
        .invoke(
            &SecretValue::new("brokered-credential-sentinel").expect("credential"),
            "api.example.test",
            "complete",
            b"{}",
        )
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    upstream.await.expect("fake upstream task");
}
#[tokio::test]
async fn loopback_adapter_rate_limits_before_connecting_upstream() {
    let adapter = LoopbackCompletionAdapter::new(
        "api.example.test",
        "127.0.0.1:1".parse().expect("address"),
        1,
    )
    .expect("adapter");
    let _occupied = Arc::clone(&adapter.concurrency)
        .acquire_owned()
        .await
        .expect("occupy adapter");
    let result = adapter
        .invoke(
            &SecretValue::new("bounded-credential").expect("credential"),
            "api.example.test",
            "complete",
            b"{}",
        )
        .await;
    assert!(matches!(result, Err(BrokerAdapterError::Retryable)));
}
