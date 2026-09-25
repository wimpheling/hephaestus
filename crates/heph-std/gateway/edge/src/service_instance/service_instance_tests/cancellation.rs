use super::fixtures::*;
use crate::{ServiceInstancePolicy, new_service_instance};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

#[tokio::test]
async fn cancellation_wins_over_a_busy_event_channel() {
    let launch = launch_with_logs(Uuid::new_v4());
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
    let mut diagnostics = handle.subscribe_diagnostics();
    let logs = handle.service_logs().expect("application log queue");
    let mut task = tokio::spawn(worker.run());
    let events = vm.events.clone();
    let stop_flood = Arc::new(AtomicBool::new(false));
    let flood_stop = Arc::clone(&stop_flood);
    let mut flood = tokio::spawn(async move {
        while !flood_stop.load(Ordering::Relaxed) {
            for _ in 0..100 {
                let _ = events.send(vm_trait::VmEvent::Log {
                    stream: vm_trait::LogStream::Stdout,
                    bytes: b"request Authorization secret".to_vec(),
                });
                tokio::task::yield_now().await;
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let snapshot = *diagnostics.borrow();
            if snapshot.lifecycle.started() || snapshot.stdout_bytes > 0 {
                break;
            }
            diagnostics.changed().await.expect("diagnostics update");
        }
    })
    .await
    .expect("event flood observed");
    drop(handle);
    let worker_result =
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut task).await {
            result.expect("worker join")
        } else {
            task.abort();
            let _ = task.await;
            panic!("worker cleanup timed out");
        };
    assert!(worker_result.is_ok());
    stop_flood.store(true, Ordering::Relaxed);
    if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut flood).await {
        result.expect("event flood");
    } else {
        flood.abort();
        let _ = flood.await;
        panic!("event flood did not stop");
    }
    assert_eq!(vm.destroys.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(logs.snapshot().queued_chunks > 0);
}

#[tokio::test]
async fn event_flood_does_not_restart_a_blocked_readiness_probe() {
    let launch = launch_with_logs(Uuid::new_v4());
    let vm = FakeVm::new(launch.spec.id.clone());
    let (client, _peer) = tokio::io::duplex(4096);
    vm.push(Box::new(client));
    let resolver = Arc::new(FakeResolver {
        cleanups: AtomicUsize::new(0),
    });
    let slow_policy = ServiceInstancePolicy::new(
        Duration::from_secs(2),
        Duration::from_millis(20),
        Duration::from_millis(500),
        Duration::from_millis(50),
    );
    let (handle, _, worker) = new_service_instance(
        launch,
        vm.clone(),
        resolver.clone(),
        "service.test",
        slow_policy,
    )
    .expect("worker");
    let logs = handle.service_logs().expect("application log queue");
    let first_open = vm.first_open.clone();
    let opened = first_open.notified();
    let mut diagnostics = handle.subscribe_diagnostics();
    let mut task = tokio::spawn(worker.run());
    tokio::time::timeout(Duration::from_secs(2), opened)
        .await
        .expect("readiness probe opened");
    assert_eq!(vm.opens.load(Ordering::Relaxed), 1);
    let events = vm.events.clone();
    let stop_flood = Arc::new(AtomicBool::new(false));
    let flood_stop = Arc::clone(&stop_flood);
    let mut flood = tokio::spawn(async move {
        while !flood_stop.load(Ordering::Relaxed) {
            for _ in 0..100 {
                let _ = events.send(vm_trait::VmEvent::Log {
                    stream: vm_trait::LogStream::Stdout,
                    bytes: b"request Authorization secret".to_vec(),
                });
                tokio::task::yield_now().await;
            }
        }
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let snapshot = *diagnostics.borrow();
            if snapshot.stdout_bytes > 0 || snapshot.lagged_events > 0 {
                break;
            }
            diagnostics.changed().await.expect("diagnostics update");
        }
    })
    .await
    .expect("event flood consumed");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if logs.snapshot().queued_chunks > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("application log flood captured");
    assert_eq!(vm.opens.load(Ordering::Relaxed), 1);
    handle.shutdown();
    let worker_result =
        if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut task).await {
            result.expect("worker join")
        } else {
            task.abort();
            let _ = task.await;
            panic!("worker cleanup timed out");
        };
    assert!(worker_result.is_ok());
    stop_flood.store(true, Ordering::Relaxed);
    if let Ok(result) = tokio::time::timeout(Duration::from_secs(5), &mut flood).await {
        result.expect("event flood");
    } else {
        flood.abort();
        let _ = flood.await;
        panic!("event flood did not stop");
    }
    assert_eq!(resolver.cleanups.load(Ordering::Relaxed), 1);
}
