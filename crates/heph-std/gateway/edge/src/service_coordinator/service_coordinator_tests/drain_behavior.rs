use super::*;

fn drain_request() -> GatewayRequest {
    GatewayRequest {
        method: http::Method::GET,
        path_and_query: String::from("/drain-check"),
        headers: http::HeaderMap::new(),
        body: bytes::Bytes::new(),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Http,
            authority: String::from("service.test"),
            client_address: "127.0.0.1".parse().expect("client address"),
            request_id: Uuid::new_v4(),
        },
    }
}

fn drain_http_policy() -> ServiceHttpPolicy {
    ServiceHttpPolicy::from_gateway_limits(GatewayLimits {
        max_request_body_bytes: 1024,
        max_response_body_bytes: 1024,
        max_request_headers: 16,
        max_response_headers: 16,
        max_path_and_query_bytes: 256,
        execution_timeout: Duration::from_secs(1),
    })
}

#[tokio::test]
// The fixture owns control and VM resources until the coordinator task joins.
#[allow(clippy::significant_drop_tightening)]
async fn drain_keeps_registry_dispatch_until_accepted_count_reaches_zero() {
    let count = default_count_state();
    count.responses.lock().expect("count responses").extend([
        Err(GatewayEdgeError::Unavailable),
        Ok(1),
        Ok(0),
    ]);
    let fixture = start_drain_fixture(count.clone(), false, false, Duration::from_secs(2)).await;
    fixture.control.request_drain();
    let mut status = fixture.status;
    timeout(Duration::from_secs(1), async {
        while *status.borrow() != GatewayServiceCoordinatorStatus::Draining {
            status.changed().await.expect("draining status");
        }
    })
    .await
    .expect("draining");
    count.started.notified().await;
    let (client, peer) = tokio::io::duplex(4096);
    fixture
        .vm
        .connections
        .lock()
        .expect("connections")
        .push_back(Box::new(client));
    tokio::spawn(respond(peer));
    let response = fixture
        .registry
        .exchange(fixture.key, drain_request(), drain_http_policy())
        .await
        .expect("accepted call remains dispatchable while draining");
    assert_eq!(response.status, http::StatusCode::OK);
    assert!(fixture.task.await.expect("coordinator join").is_ok());
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(
        fixture
            .targets
            .count
            .responses
            .lock()
            .expect("responses")
            .len(),
        0
    );
    assert_eq!(
        fixture.ownership.events.lock().expect("events").as_slice(),
        [
            "starting", "ready", "promote", "draining", "stopping", "cleaned"
        ]
    );
}

#[tokio::test]
// The fixture owns control and VM resources until the coordinator task joins.
#[allow(clippy::significant_drop_tightening)]
async fn drain_conflict_consumes_request_without_spinning_or_leaving_ready() {
    let count = default_count_state();
    let fixture = start_drain_fixture(count.clone(), true, false, Duration::from_secs(1)).await;
    fixture.control.request_drain();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert_eq!(
        *fixture.status.borrow(),
        GatewayServiceCoordinatorStatus::Ready
    );
    assert!(count.responses.lock().expect("count responses").is_empty());
    assert_eq!(
        fixture
            .ownership
            .events
            .lock()
            .expect("events")
            .iter()
            .filter(|event| **event == "draining")
            .count(),
        1
    );
    fixture
        .ownership
        .drain_conflict
        .store(false, Ordering::Relaxed);
    fixture.control.request_drain();
    timeout(Duration::from_secs(1), async {
        while *fixture.status.borrow() != GatewayServiceCoordinatorStatus::Stopped {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("second drain retires instance");
    assert!(fixture.task.await.expect("coordinator join").is_ok());
}

#[tokio::test]
// The fixture owns control and VM resources until the coordinator task joins.
#[allow(clippy::significant_drop_tightening)]
async fn drain_deadline_retires_without_failure_backoff() {
    let count = default_count_state();
    count.blocked.store(true, Ordering::Relaxed);
    let fixture = start_drain_fixture(count.clone(), false, false, Duration::from_millis(40)).await;
    fixture.control.request_drain();
    count.started.notified().await;
    assert!(fixture.task.await.expect("coordinator join").is_ok());
    assert!(
        fixture
            .targets
            .count
            .responses
            .lock()
            .expect("count responses")
            .is_empty()
    );
}

#[tokio::test]
#[allow(clippy::significant_drop_tightening)]
// The fixture retains the coordinator control and VM until its joined task
// proves the blocked durable transition settled within the total budget.
async fn drain_deadline_includes_mark_draining_transition() {
    let count = default_count_state();
    let fixture = start_drain_fixture(count, false, true, Duration::from_millis(40)).await;
    fixture.control.request_drain();
    fixture.ownership.drain_started.notified().await;
    assert!(
        timeout(Duration::from_secs(1), fixture.task)
            .await
            .expect("mark draining deadline")
            .expect("coordinator join")
            .is_ok()
    );
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
}

#[tokio::test]
// The fixture owns control and VM resources until the coordinator task joins.
#[allow(clippy::significant_drop_tightening)]
async fn lease_loss_during_blocked_drain_count_still_cleans() {
    let count = default_count_state();
    count.blocked.store(true, Ordering::Relaxed);
    let fixture = start_drain_fixture(count.clone(), false, false, Duration::from_secs(2)).await;
    fixture.control.request_drain();
    count.started.notified().await;
    fixture.ownership.renew_fails.store(true, Ordering::Relaxed);
    let result = timeout(Duration::from_secs(1), fixture.task)
        .await
        .expect("lease loss cleanup")
        .expect("coordinator join");
    assert!(result.is_err());
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
}

#[tokio::test]
#[allow(clippy::significant_drop_tightening)]
// The blocked count future remains owned while cancellation drives normal
// teardown, so the VM cannot be abandoned by the parent request.
async fn cancellation_during_blocked_drain_count_still_cleans() {
    let count = default_count_state();
    count.blocked.store(true, Ordering::Relaxed);
    let fixture = start_drain_fixture(count, false, false, Duration::from_secs(2)).await;
    fixture.control.request_drain();
    fixture.targets.count.started.notified().await;
    fixture.control.cancel();
    assert!(
        timeout(Duration::from_secs(1), fixture.task)
            .await
            .expect("cancellation cleanup")
            .expect("coordinator join")
            .is_ok()
    );
    assert_eq!(fixture.vm.destroys.load(Ordering::Relaxed), 1);
}
