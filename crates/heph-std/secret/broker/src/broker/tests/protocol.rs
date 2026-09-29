// Secret broker test scenarios.
use super::support::*;

#[tokio::test]
async fn framed_transport_never_echoes_runtime_credential() {
    let temporary = TempDir::new().expect("temporary directory");
    let socket = temporary.path().join("broker.sock");
    let executor = Arc::new(RecordingExecutor {
        seen: Mutex::new(Vec::new()),
    });
    let server = BrokerServer::bind(socket.clone(), executor.clone()).expect("bind broker");
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(server.serve(cancellation.clone()));
    let mut client = UnixStream::connect(&socket).await.expect("connect broker");
    let sentinel = b"runtime-credential-sentinel".to_vec();
    let request = WireBrokerRequest {
        credential: sentinel.clone(),
        run_id: RunId::new(),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("complete"),
        body: b"request".to_vec(),
    };
    let encoded = serde_json::to_vec(&request).expect("encode request");
    write_frame(&mut client, &encoded)
        .await
        .expect("write request");
    let response_length = client.read_u32().await.expect("response length");
    let mut response = vec![0_u8; response_length as usize];
    client
        .read_exact(&mut response)
        .await
        .expect("response payload");
    assert!(
        !response
            .windows(sentinel.len())
            .any(|value| value == sentinel)
    );
    let response: WireBrokerResponse = serde_json::from_slice(&response).expect("decode response");
    assert_eq!(response.status, WireBrokerStatus::Succeeded);
    assert_eq!(
        executor.seen.lock().expect("recording lock").as_slice(),
        sentinel
    );
    cancellation.cancel();
    task.await.expect("broker task").expect("broker shutdown");
}
#[tokio::test]
async fn released_client_round_trips_only_the_sanitized_broker_response() {
    let temporary = TempDir::new().expect("temporary directory");
    let socket = temporary.path().join("broker.sock");
    let recorder = Arc::new(RecordingExecutor {
        seen: Mutex::new(Vec::new()),
    });
    let executor: Arc<dyn BrokerExecutor> = recorder.clone();
    let server = BrokerServer::bind(socket.clone(), Arc::clone(&executor)).expect("bind broker");
    let cancellation = CancellationToken::new();
    let task = tokio::spawn(server.serve(cancellation.clone()));
    let sentinel = b"released-client-credential-sentinel".to_vec();
    let request = WireBrokerRequest {
        credential: sentinel.clone(),
        run_id: RunId::new(),
        slot: String::from("model"),
        destination: String::from("api.example.test"),
        operation: String::from("https_v1"),
        body: serde_json::to_vec(&BrokeredHttpsRequest {
            rule_id: uuid::Uuid::new_v4(),
            method: BrokeredHttpsMethod::Get,
            path_and_query: String::from("/v1/health"),
            headers: Vec::new(),
            body: Vec::new(),
        })
        .expect("encode HTTPS request"),
    };
    let response = tokio::task::spawn_blocking(move || {
        let stream = std::os::unix::net::UnixStream::connect(socket).expect("connect broker");
        BrokeredHttpsClient::new(stream)
            .call(&request)
            .expect("released broker client call")
    })
    .await
    .expect("released client task");
    assert_eq!(response.status, WireBrokerStatus::Succeeded);
    assert!(
        !response
            .body
            .windows(sentinel.len())
            .any(|window| window == sentinel)
    );
    assert_eq!(
        recorder.seen.lock().expect("recording lock").as_slice(),
        sentinel
    );
    cancellation.cancel();
    task.await.expect("broker task").expect("broker shutdown");
}
