use super::{fixtures::*, lifecycle::wait_for_state};
use crate::{ServiceWorkerState, new_service_instance};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[tokio::test]
async fn application_log_capture_is_opt_in_and_starts_before_readiness() {
    let launch = launch_with_logs(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    vm.emit_start_log.store(true, Ordering::Relaxed);
    let (client, peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    tokio::spawn(response_peer(peer, 200));
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (handle, mut state, worker) =
        new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
    let logs = handle.service_logs().expect("application log queue");
    let task = tokio::spawn(worker.run());
    wait_for_state(&mut state, ServiceWorkerState::Ready).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if logs.snapshot().queued_chunks > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("start log captured");
    let records = logs.drain(8, 1024);
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].bytes, b"before-readiness");
    handle.shutdown();
    assert!(task.await.expect("worker join").is_ok());
}

#[test]
fn disabled_log_capture_has_no_raw_queue() {
    let launch = launch(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let (handle, _, _) =
        new_service_instance(launch, vm, resolver, "service.test", policy()).expect("worker");
    assert!(handle.service_logs().is_none());
}
