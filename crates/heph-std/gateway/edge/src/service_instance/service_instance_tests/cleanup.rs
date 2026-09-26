use super::{fixtures::*, lifecycle::wait_for_state};
use crate::{
    GatewayServiceFailure, GatewayServiceFailureCode, ServiceInstanceError, ServiceWorkerState,
    new_service_instance,
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;
use vm_trait::VmExit;

#[tokio::test]
async fn stop_failure_still_destroys_and_cleans_materializer() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    vm.stop_ok.store(false, Ordering::Relaxed);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 200));
    let (handle, mut state, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        policy(),
    )
    .expect("worker");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Ready).await;
    handle.shutdown();
    assert!(task.await.expect("worker join").is_ok());
    assert_eq!(handle.failure(), None);
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn vm_exit_triggers_cleanup_after_readiness() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 200));
    let (handle, mut state, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        policy(),
    )
    .expect("worker");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Ready).await;
    vm.exit_with(VmExit {
        code: Some(17),
        signal: None,
    });
    assert_eq!(
        task.await.expect("worker join"),
        Err(ServiceInstanceError::UnexpectedExit)
    );
    assert_eq!(
        handle.failure(),
        Some(GatewayServiceFailure {
            code: GatewayServiceFailureCode::UnexpectedExit,
            exit_code: Some(17),
            exit_signal: None,
        })
    );
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn exit_signal_is_retained_when_destroy_fails() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    vm.destroy_ok.store(false, Ordering::Relaxed);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 200));
    let (handle, mut state, worker) =
        new_service_instance(launch, vm.clone(), resolver, "service.test", policy())
            .expect("worker");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Ready).await;
    vm.exit_with(VmExit {
        code: None,
        signal: Some(9),
    });
    assert_eq!(
        task.await.expect("worker join"),
        Err(ServiceInstanceError::CleanupIncomplete)
    );
    assert_eq!(
        handle.failure(),
        Some(GatewayServiceFailure {
            code: GatewayServiceFailureCode::UnexpectedExit,
            exit_code: None,
            exit_signal: Some(9),
        })
    );
}

#[tokio::test]
async fn malformed_exit_metadata_is_redacted() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 200));
    let (handle, mut state, worker) =
        new_service_instance(launch, vm.clone(), resolver, "service.test", policy())
            .expect("worker");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Ready).await;
    vm.exit_with(VmExit {
        code: Some(999),
        signal: Some(9),
    });
    assert_eq!(
        task.await.expect("worker join"),
        Err(ServiceInstanceError::UnexpectedExit)
    );
    assert_eq!(
        handle.failure(),
        Some(GatewayServiceFailure {
            code: GatewayServiceFailureCode::UnexpectedExit,
            exit_code: None,
            exit_signal: None,
        })
    );
}
