use std::{sync::Arc, time::Duration};

use bytes::Bytes;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use vm_trait::{LogStream, PrivateHttpRequest, StopMode, VmEvent, VmProvider};

use crate::{boot::BootContext, support::*};

// This phase keeps one real guest lifecycle sequence together so cleanup and
// cross-step assertions remain auditable.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(context: &mut BootContext) {
    let rootfs = context.rootfs.clone();
    let sqlite_disk = context.sqlite_disk.clone();
    let sqlite_uuid = context.sqlite_uuid.clone();
    let repository = context.repository.clone();
    let workspace = context.workspace.clone();
    let runtime_root = context.runtime_root.clone();
    let sqlite_disk_for_assertion = context.sqlite_disk_for_assertion.clone();
    let cgroup_root_for_assertion = context.cgroup_root.clone();
    let expected_limits = &context.expected_limits;
    let provider = &context.provider;
    let mut secret_mount = context.secret_mount.take().expect("primary secret mount");
    let secret_path = context.secret_path.clone();
    let spec = integration_spec(
        "integration-primary",
        rootfs.clone(),
        sqlite_disk.clone(),
        &sqlite_uuid,
        repository.clone(),
        workspace.clone(),
        Some(secret_mount.vm_mount()),
    );

    let vm = provider.provision(spec).await.expect("provision VM");
    let vm_id = vm.id().0.clone();
    let mut events = vm.subscribe_events();
    let first = Arc::clone(&vm);
    let second = Arc::clone(&vm);
    let (first, second) = tokio::join!(first.start(), second.start());
    first.expect("first concurrent start");
    second.expect("second concurrent start");
    assert_cgroup_limits(&cgroup_root_for_assertion.join(&vm_id), expected_limits);

    let mut markers = String::new();
    let mut stderr_seen = false;
    let mut metric_seen = false;
    let mut ready_seen = false;
    let exit = tokio::time::timeout(Duration::from_secs(90), async {
        loop {
            match events.recv().await.expect("ordered VM event") {
                VmEvent::Started { ingress } => {
                    assert_ne!(ingress[0].host_port, 0);
                }
                VmEvent::Ready => ready_seen = true,
                VmEvent::Log { stream, bytes } => {
                    if matches!(stream, LogStream::Stderr) {
                        stderr_seen = true;
                    }
                    markers.push_str(&String::from_utf8_lossy(&bytes));
                }
                VmEvent::Metric(metric) if metric.name == "heph_init.ready" => {
                    metric_seen = true;
                }
                VmEvent::Exited(exit) => break exit,
                _ => {}
            }
        }
    })
    .await
    .expect("guest completion timeout");

    assert_eq!(exit.code, Some(0), "guest output:\n{markers}");
    for marker in [
        "sqlite=ok",
        "mounts=ok",
        "secrets=ok",
        "dns=ok",
        "tcp=ok",
        "udp=ok",
        "stderr=ok",
    ] {
        assert!(markers.contains(marker), "missing guest marker {marker}");
    }
    assert!(ready_seen, "missing Ready event");
    assert!(stderr_seen, "missing stderr event");
    assert!(metric_seen, "missing metric event");
    let previous_sqlite_rows = sqlite_previous_rows(&markers);
    assert_eq!(vm.wait().await.expect("cached exit"), exit);
    vm.stop(StopMode::Graceful {
        timeout: Duration::from_secs(2),
    })
    .await
    .expect("idempotent graceful stop");
    vm.destroy().await.expect("complete cleanup");
    vm.destroy().await.expect("idempotent cleanup");
    secret_mount
        .mark_guest_destroyed()
        .expect("confirm primary guest destruction");
    secret_mount
        .destroy()
        .expect("destroy exact raw secret mount");
    assert!(
        !secret_path.exists(),
        "raw secret mount survived guest destruction"
    );
    assert!(!runtime_root.join(&vm_id).exists());
    assert!(!cgroup_root_for_assertion.join(&vm_id).exists());

    let rollback = provider
        .provision(state_probe_spec(
            "integration-state-rollback",
            rootfs.clone(),
            sqlite_disk.clone(),
            &sqlite_uuid,
            "--state-rollback",
        ))
        .await
        .expect("provision rollback update VM");
    let rollback_id = rollback.id().0.clone();
    let mut rollback_events = rollback.subscribe_events();
    rollback.start().await.expect("start rollback update VM");
    let (rollback_markers, rollback_exit) = collect_logs_until_any_exit(&mut rollback_events).await;
    assert_eq!(rollback_exit.code, Some(23));
    assert!(rollback_markers.contains("sqlite-rollback=ok"));
    rollback
        .destroy()
        .await
        .expect("destroy rollback update VM");
    assert!(!runtime_root.join(&rollback_id).exists());
    assert!(!cgroup_root_for_assertion.join(&rollback_id).exists());

    let persisted = provider
        .provision(integration_spec(
            "integration-persistence",
            rootfs.clone(),
            sqlite_disk,
            &sqlite_uuid,
            repository,
            workspace,
            None,
        ))
        .await
        .expect("provision persistence VM");
    let persisted_id = persisted.id().0.clone();
    let mut persisted_events = persisted.subscribe_events();
    persisted.start().await.expect("start persistence VM");
    let persisted_markers = collect_logs_until_exit(&mut persisted_events).await;
    assert_eq!(
        sqlite_previous_rows(&persisted_markers),
        previous_sqlite_rows + 1,
        "SQLite contents did not persist across VM boots"
    );
    persisted.destroy().await.expect("destroy persistence VM");
    assert!(
        sqlite_disk_for_assertion.is_file(),
        "destroy removed caller-owned state disk"
    );
    assert!(!runtime_root.join(&persisted_id).exists());

    let gateway = provider
        .provision(private_http_spec(rootfs.clone()))
        .await
        .expect("provision private HTTP gateway VM");
    let gateway_id = gateway.id().0.clone();
    gateway
        .start()
        .await
        .expect("start private HTTP gateway VM");
    let mut request_headers = HeaderMap::new();
    request_headers.insert("content-type", HeaderValue::from_static("text/plain"));
    let response = gateway
        .invoke_private_http(PrivateHttpRequest {
            method: Method::POST,
            path_and_query: "/gateway/proof?mode=real".to_owned(),
            headers: request_headers,
            body: Bytes::from_static(b"gateway-real-vm-request"),
        })
        .await
        .expect("complete private HTTP request through real guest control channel");
    assert_eq!(response.status, StatusCode::CREATED);
    assert_eq!(
        response.headers.get("content-type"),
        Some(&HeaderValue::from_static("text/plain"))
    );
    assert_eq!(
        response.body,
        Bytes::from_static(b"gateway-real-vm-response")
    );
    gateway
        .destroy()
        .await
        .expect("destroy private HTTP gateway VM");
    assert!(!runtime_root.join(&gateway_id).exists());
    assert!(!cgroup_root_for_assertion.join(&gateway_id).exists());
    assert!(!cgroup_root_for_assertion.join(&persisted_id).exists());
}
