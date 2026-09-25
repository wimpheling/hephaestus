use super::fixtures::*;
use crate::{
    GatewayServiceFailureCode, ServiceInstanceError, ServiceInstancePolicy, ServiceProbeError,
    ServiceWorkerState, new_service_instance,
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::watch;
use uuid::Uuid;

pub(super) async fn wait_for_state(
    receiver: &mut watch::Receiver<ServiceWorkerState>,
    expected: ServiceWorkerState,
) {
    while *receiver.borrow() != expected {
        receiver.changed().await.expect("worker state");
    }
}

#[tokio::test]
async fn readiness_is_required_before_ready_and_cleanup_is_exact() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 503));
    let (handle, state, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        policy(),
    )
    .expect("worker");
    let task = tokio::spawn(worker.run());
    assert_eq!(handle.health().await, Err(ServiceInstanceError::NotReady));
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_ne!(*state.borrow(), ServiceWorkerState::Ready);
    let result = task.await.expect("worker join");
    assert!(matches!(
        result,
        Err(ServiceInstanceError::StartupTimeout | ServiceInstanceError::StartupFailed)
    ));
    assert_eq!(
        handle.failure().map(|failure| failure.code),
        Some(GatewayServiceFailureCode::Readiness)
    );
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(handle.diagnostics().lifecycle.started());
    assert_eq!(
        handle.diagnostics().worker_state,
        ServiceWorkerState::Stopped
    );
    drop(handle);
}

#[tokio::test]
async fn ready_health_failure_keeps_instance_alive_until_shutdown() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    for status in [200, 503] {
        let (client, peer) = tokio::io::duplex(4096);
        vm.push(Box::new(client));
        tokio::spawn(response_peer(peer, status));
    }
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
    assert!(matches!(
        handle.health().await,
        Err(ServiceInstanceError::HealthProbe(
            ServiceProbeError::NonSuccess(_)
        ))
    ));
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 0);
    handle.shutdown();
    assert!(task.await.expect("worker join").is_ok());
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}

#[tokio::test]
async fn destroy_failure_does_not_claim_materializer_cleanup() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    vm.destroy_ok.store(false, Ordering::Relaxed);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (handle, mut state, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        policy(),
    )
    .expect("worker");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Probing).await;
    drop(handle);
    assert_eq!(
        task.await.expect("worker join"),
        Err(ServiceInstanceError::CleanupIncomplete)
    );
    assert_eq!(*state.borrow(), ServiceWorkerState::CleanupIncomplete);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn startup_deadline_includes_blocked_start() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    *vm.start_delay.lock().expect("start delay lock") = Duration::from_millis(200);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let short = ServiceInstancePolicy::new(
        Duration::from_millis(20),
        Duration::from_millis(1),
        Duration::from_millis(10),
        Duration::from_millis(50),
    );
    let (handle, _, worker) =
        new_service_instance(launch, vm.clone(), resolver.clone(), "service.test", short)
            .expect("worker");
    let task = tokio::spawn(worker.run());
    assert_eq!(
        task.await.expect("worker join"),
        Err(ServiceInstanceError::StartupTimeout)
    );
    assert_eq!(
        handle.failure().map(|failure| failure.code),
        Some(GatewayServiceFailureCode::Startup)
    );
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    drop(handle);
}

#[tokio::test]
async fn startup_failure_is_classified_as_startup() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    vm.start_ok.store(false, Ordering::Relaxed);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (handle, _, worker) =
        new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
    assert_eq!(
        tokio::spawn(worker.run()).await.expect("worker join"),
        Err(ServiceInstanceError::StartupFailed)
    );
    assert_eq!(
        handle.failure().map(|failure| failure.code),
        Some(GatewayServiceFailureCode::Startup)
    );
}

#[tokio::test]
async fn dropping_last_handle_cancels_blocked_start() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    *vm.start_delay.lock().expect("start delay lock") = Duration::from_millis(200);
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (handle, _, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        policy(),
    )
    .expect("worker");
    let task = tokio::spawn(worker.run());
    tokio::time::sleep(Duration::from_millis(5)).await;
    drop(handle);
    assert!(task.await.expect("worker join").is_ok());
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}
