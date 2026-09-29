use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use gateway_domain::{GatewayServiceConfig, ServiceProbePath};
use gateway_edge::{
    GatewayServiceFailure, GatewayServiceFailureCode, GatewayServiceIdentity, GatewayServiceLaunch,
    ServiceInstancePolicy, ServiceWorkerState, new_service_instance,
};
use http::StatusCode;
use tokio::io::AsyncReadExt;
use vm_trait::{NetworkMode, VmError, VmProvider};

use crate::{boot::BootContext, support::*};

// This phase keeps one real guest lifecycle sequence together so cleanup and
// cross-step assertions remain auditable.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(context: &BootContext) {
    let rootfs = context.rootfs.clone();
    let runtime_root = context.runtime_root.clone();
    let cgroup_root_for_assertion = context.cgroup_root.clone();
    let provider = &context.provider;
    let service_spec = private_service_spec(
        rootfs.clone(),
        format!("integration-private-service-{}", std::process::id()),
    );
    assert!(service_spec.runtime_authority.is_none());
    assert!(matches!(service_spec.network, NetworkMode::Disabled));
    assert!(
        !service_spec
            .labels
            .contains_key("hephaestus.gateway.handler-contract")
    );
    let service = provider
        .provision(service_spec)
        .await
        .expect("provision persistent private service VM");
    let service_id = service.id().0.clone();
    let mut service_events = service.subscribe_events();
    service
        .start()
        .await
        .expect("start persistent private service VM");
    wait_for_service_isolation(&mut service_events).await;

    let ready = poll_private_service(&service, "/readyz").await;
    assert_eq!(ready.0, 200, "service readiness response: {ready:?}");
    assert_eq!(ready.1, b"ready");
    assert_eq!(
        poll_private_service(&service, "/healthz").await,
        (200, b"healthy".to_vec())
    );

    let readiness_probe = poll_private_service_probe(&service, "/readyz").await;
    assert_eq!(readiness_probe.status, StatusCode::OK);
    let health_probe = poll_private_service_probe(&service, "/healthz").await;
    assert_eq!(health_probe.status, StatusCode::OK);

    let identity_one = parse_adapter_service_identity(
        &private_service_adapter_request(&service, "/identity").await,
    );
    let identity_two = parse_adapter_service_identity(
        &private_service_adapter_request(&service, "/identity").await,
    );
    assert_eq!(identity_one["pid"], identity_two["pid"]);
    assert_eq!(identity_one["startup_id"], identity_two["startup_id"]);

    let delayed_vm = Arc::clone(&service);
    let delayed =
        tokio::spawn(async move { private_service_request(&delayed_vm, "/delay/1500").await });
    wait_for_log(&mut service_events, "private-service-delay=started").await;
    let health = tokio::time::timeout(
        Duration::from_secs(1),
        private_service_request(&service, "/healthz"),
    )
    .await
    .expect("health request did not complete while delayed request was active")
    .expect("concurrent health response");
    assert_eq!(health, (200, b"healthy".to_vec()));
    assert!(
        !delayed.is_finished(),
        "delayed request completed too early"
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), delayed)
            .await
            .expect("delayed service response timeout")
            .expect("delayed service task")
            .expect("delayed service response")
            .0,
        200
    );

    let held_one = service
        .open_private_service_connection()
        .await
        .expect("first service capacity connection");
    let held_two = service
        .open_private_service_connection()
        .await
        .expect("second service capacity connection");
    assert!(matches!(
        service.open_private_service_connection().await,
        Err(VmError::Unavailable { .. })
    ));
    drop(held_one);
    drop(held_two);

    let active = service
        .open_private_service_connection()
        .await
        .expect("active service connection before destroy");
    service
        .destroy()
        .await
        .expect("destroy persistent service VM");
    let mut active = active;
    let mut closed = [0_u8; 1];
    let close_result = tokio::time::timeout(Duration::from_secs(5), active.read(&mut closed))
        .await
        .expect("destroy closes active service stream");
    assert!(matches!(close_result, Err(_) | Ok(0)));
    assert!(!runtime_root.join(&service_id).exists());
    assert!(!cgroup_root_for_assertion.join(&service_id).exists());

    let worker_identity = GatewayServiceIdentity {
        instance_id: uuid::Uuid::new_v4(),
        gateway_id: uuid::Uuid::new_v4(),
        revision_id: uuid::Uuid::new_v4(),
    };
    let worker_spec = private_service_spec(
        rootfs.clone(),
        format!("gateway-service-{}", worker_identity.instance_id),
    );
    let worker_launch = GatewayServiceLaunch {
        identity: worker_identity,
        service: GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/readyz").expect("worker readiness path"),
            ServiceProbePath::parse("/healthz").expect("worker health path"),
        )
        .expect("worker service declaration"),
        spec: worker_spec,
    };
    let worker_vm = provider
        .provision(worker_launch.spec.clone())
        .await
        .expect("provision prepared service worker VM");
    let worker_vm_id = worker_vm.id().0.clone();
    let worker_resolver = Arc::new(IntegrationServiceResolver {
        expected: worker_identity,
        cleanups: AtomicUsize::new(0),
    });
    let (worker_handle, mut worker_state, worker) = new_service_instance(
        worker_launch,
        Arc::clone(&worker_vm),
        worker_resolver.clone(),
        "service.internal",
        ServiceInstancePolicy::new(
            Duration::from_secs(30),
            Duration::from_millis(100),
            Duration::from_secs(3),
            Duration::from_secs(5),
        ),
    )
    .expect("construct prepared service worker");
    let worker_task = tokio::spawn(worker.run());
    tokio::time::timeout(Duration::from_secs(60), async {
        while *worker_state.borrow() != ServiceWorkerState::Ready {
            worker_state
                .changed()
                .await
                .expect("prepared service worker state");
        }
    })
    .await
    .expect("prepared service worker readiness timeout");
    println!("REAL_PREPARED_SERVICE_WORKER_READY=1");
    assert_eq!(
        worker_handle
            .health()
            .await
            .expect("prepared service worker health"),
        StatusCode::OK
    );
    println!("REAL_PREPARED_SERVICE_WORKER_HEALTH=1");
    let first_identity = parse_adapter_service_identity(
        &private_service_adapter_request(&worker_vm, "/identity").await,
    );
    assert_eq!(
        private_service_request(&worker_vm, "/crash")
            .await
            .expect("prepared service worker crash response"),
        (503, b"crashing".to_vec())
    );
    let worker_result = tokio::time::timeout(Duration::from_secs(15), worker_task)
        .await
        .expect("prepared service worker cleanup timeout")
        .expect("prepared service worker join")
        .expect_err("prepared service worker exit must be reported");
    assert_eq!(
        worker_result,
        gateway_edge::ServiceInstanceError::UnexpectedExit
    );
    assert_eq!(
        worker_handle.failure(),
        Some(GatewayServiceFailure {
            code: GatewayServiceFailureCode::UnexpectedExit,
            exit_code: Some(42),
            exit_signal: None,
        })
    );
    assert_eq!(worker_resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(!runtime_root.join(&worker_vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&worker_vm_id).exists());
    println!("REAL_PREPARED_SERVICE_WORKER_CRASH=1");
    println!("REAL_PREPARED_SERVICE_WORKER_CLEANED=1");

    let replacement_identity = GatewayServiceIdentity {
        instance_id: uuid::Uuid::new_v4(),
        ..worker_identity
    };
    let replacement_launch = GatewayServiceLaunch {
        identity: replacement_identity,
        service: GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/readyz").expect("replacement readiness path"),
            ServiceProbePath::parse("/healthz").expect("replacement health path"),
        )
        .expect("replacement service declaration"),
        spec: private_service_spec(
            rootfs.clone(),
            format!("gateway-service-{}", replacement_identity.instance_id),
        ),
    };
    let replacement_vm = provider
        .provision(replacement_launch.spec.clone())
        .await
        .expect("provision replacement service worker VM");
    let replacement_vm_id = replacement_vm.id().0.clone();
    assert_ne!(replacement_vm_id, worker_vm_id);
    let replacement_resolver = Arc::new(IntegrationServiceResolver {
        expected: replacement_identity,
        cleanups: AtomicUsize::new(0),
    });
    let (replacement_handle, mut replacement_state, replacement_worker) = new_service_instance(
        replacement_launch,
        Arc::clone(&replacement_vm),
        replacement_resolver.clone(),
        "service.internal",
        ServiceInstancePolicy::new(
            Duration::from_secs(30),
            Duration::from_millis(100),
            Duration::from_secs(3),
            Duration::from_secs(5),
        ),
    )
    .expect("construct replacement service worker");
    let replacement_task = tokio::spawn(replacement_worker.run());
    tokio::time::timeout(Duration::from_secs(60), async {
        while *replacement_state.borrow() != ServiceWorkerState::Ready {
            replacement_state
                .changed()
                .await
                .expect("replacement service worker state");
        }
    })
    .await
    .expect("replacement service worker readiness timeout");
    let replacement_identity_response = parse_adapter_service_identity(
        &private_service_adapter_request(&replacement_vm, "/identity").await,
    );
    assert_ne!(
        first_identity["startup_id"],
        replacement_identity_response["startup_id"]
    );
    replacement_handle.shutdown();
    tokio::time::timeout(Duration::from_secs(15), replacement_task)
        .await
        .expect("replacement service worker cleanup timeout")
        .expect("replacement service worker join")
        .expect("replacement service worker cleanup");
    assert_eq!(replacement_handle.failure(), None);
    assert_eq!(replacement_resolver.cleanups.load(Ordering::Relaxed), 1);
    assert!(!runtime_root.join(&replacement_vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&replacement_vm_id).exists());
    println!("REAL_PREPARED_SERVICE_WORKER_REPLACED=1");
}
