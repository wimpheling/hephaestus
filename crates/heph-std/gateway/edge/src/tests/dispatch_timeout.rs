use super::*;

#[tokio::test]
async fn timeout_cleans_up_the_one_shot_vm_after_the_request_future_is_cancelled() {
    let route = route();
    let provider = FakeProvider::new().with_private_http_responder(Arc::new(SlowPrivateHandler));
    let handler = PrivateHttpVmGatewayHandler::new(FakeGatewayLauncher {
        provider: provider.clone(),
    });
    let dispatcher = GatewayDispatcher::new(Resolver(route.clone()), handler, Recorder);
    assert_eq!(
        dispatcher
            .dispatch(request("/gateway/echo"))
            .await
            .response
            .status,
        StatusCode::GATEWAY_TIMEOUT
    );
    // Provisioning the same deterministic VM ID succeeds only after the
    // detached cleanup task has destroyed the timed-out instance.
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match provider.provision(gateway_vm_spec(&route)).await {
                Ok(instance) => {
                    instance.destroy().await.expect("destroy replacement VM");
                    return;
                }
                Err(VmError::AlreadyExists(_)) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected replacement provisioning error: {error}"),
            }
        }
    })
    .await
    .expect("timed-out gateway VM was cleaned up");
}
