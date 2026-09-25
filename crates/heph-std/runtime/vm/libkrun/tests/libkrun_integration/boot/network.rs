use std::{
    fs, io,
    net::{IpAddr, Ipv4Addr},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, Instant},
};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use vm_trait::{
    NetworkMode, PortForward, PortProtocol, RuntimeAuthorityBootstrap, StopMode, VmError, VmMount,
    VmProvider,
};

use crate::{boot::BootContext, support::*};

// This phase keeps one real guest lifecycle sequence together so cleanup and
// cross-step assertions remain auditable.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
pub async fn run(context: &mut BootContext) {
    let provider = &context.provider;
    let rootfs = context.rootfs.clone();
    let mount_root = context.mount_root.clone();
    let runtime_root = context.runtime_root.clone();
    let cgroup_root_for_assertion = context.cgroup_root.clone();
    let broker_session = context.broker_session;
    let broker_credential = context.broker_credential;
    let broker_shutdown = context.broker_shutdown.take().expect("broker shutdown");
    let broker_task = context.broker_task.take().expect("broker task");
    let timeout_provider = &context.timeout_provider;
    // This is the VM half of the durable mailbox journey: the same sealed
    // control mount produced by `run-runtime-local` reaches a real libkrun
    // guest as a generic envelope plus exact opaque body, never via NATS.
    let mailbox_root = mount_root.join("mailbox-control");
    fs::create_dir(&mailbox_root).expect("create mailbox control root");
    fs::write(
        mailbox_root.join("mailbox-event.json"),
        r#"{"schema_version":1,"method":"POST","route":"/mailbox/libkrun-proof","body_path":"/run/hephaestus/mailbox-body"}"#,
    )
    .expect("write mailbox envelope");
    fs::write(
        mailbox_root.join("mailbox-body"),
        b"real-libkrun-mailbox-body",
    )
    .expect("write opaque mailbox body");
    fs::set_permissions(&mailbox_root, fs::Permissions::from_mode(0o555))
        .expect("seal mailbox control root");
    let mut mailbox_spec = mode_spec(
        rootfs.clone(),
        "integration-mailbox-control",
        "--expect-mailbox",
        NetworkMode::Disabled,
    );
    mailbox_spec.mounts.push(VmMount {
        tag: "mailbox-control".to_owned(),
        host_path: mailbox_root,
        guest_path: PathBuf::from("/run/hephaestus"),
        read_only: true,
    });
    let mailbox = provider
        .provision(mailbox_spec)
        .await
        .expect("provision mailbox control VM");
    let mailbox_id = mailbox.id().0.clone();
    let mut mailbox_events = mailbox.subscribe_events();
    mailbox.start().await.expect("start mailbox control VM");
    assert!(
        collect_logs_until_exit(&mut mailbox_events)
            .await
            .contains("mailbox=ok")
    );
    mailbox.destroy().await.expect("destroy mailbox control VM");
    assert!(!runtime_root.join(&mailbox_id).exists());
    assert!(!cgroup_root_for_assertion.join(&mailbox_id).exists());

    let mut broker_spec = mode_spec(
        rootfs.clone(),
        "integration-broker-only-no-ip-bypass",
        "--expect-broker-only",
        NetworkMode::BrokerOnly,
    );
    broker_spec.runtime_authority = Some(RuntimeAuthorityBootstrap::new(
        broker_session,
        1,
        broker_credential,
    ));
    let broker_only = provider
        .provision(broker_spec)
        .await
        .expect("provision broker-only VM");
    let broker_only_id = broker_only.id().0.clone();
    let mut broker_only_events = broker_only.subscribe_events();
    broker_only.start().await.expect("start broker-only VM");
    let broker_only_markers = collect_logs_until_exit(&mut broker_only_events).await;
    assert!(broker_only_markers.contains("network-disabled=ok"));
    assert!(broker_only_markers.contains("broker-vsock=ok"));
    broker_shutdown.cancel();
    broker_task
        .await
        .expect("broker server task")
        .expect("broker server exit");
    broker_only.destroy().await.expect("destroy broker-only VM");
    assert!(!runtime_root.join(&broker_only_id).exists());
    assert!(!cgroup_root_for_assertion.join(&broker_only_id).exists());

    let http = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-http",
            "--serve-http",
            NetworkMode::UserMode {
                ingress: vec![PortForward {
                    protocol: PortProtocol::Tcp,
                    bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    host_port: 0,
                    guest_port: 8080,
                }],
            },
        ))
        .await
        .expect("provision HTTP VM");
    let http_id = http.id().0.clone();
    let mut http_events = http.subscribe_events();
    http.start().await.expect("start HTTP VM");
    let host_port = ready_http_host_port(&mut http_events).await;
    let mut connection = tokio::net::TcpStream::connect((Ipv4Addr::LOCALHOST, host_port))
        .await
        .expect("connect through passt forwarding");
    connection
        .write_all(b"GET / HTTP/1.1\r\nHost: guest\r\nConnection: close\r\n\r\n")
        .await
        .expect("write forwarded HTTP request");
    let mut response = Vec::new();
    if let Err(error) = connection.read_to_end(&mut response).await {
        assert_eq!(
            error.kind(),
            io::ErrorKind::ConnectionReset,
            "read forwarded HTTP response: {error}"
        );
    }
    assert!(response.starts_with(b"HTTP/1.1 200 OK"));
    assert!(response.ends_with(b"\r\n\r\nok"));
    assert_eq!(http.wait().await.expect("HTTP guest exit").code, Some(0));
    http.destroy().await.expect("destroy HTTP VM");
    assert!(!runtime_root.join(&http_id).exists());
    assert!(!cgroup_root_for_assertion.join(&http_id).exists());

    let ignored = provider
        .provision(mode_spec(
            rootfs.clone(),
            "integration-ignore-cancellation",
            "--ignore-cancellation",
            NetworkMode::Disabled,
        ))
        .await
        .expect("provision non-cooperative VM");
    let ignored_id = ignored.id().0.clone();
    let mut ignored_events = ignored.subscribe_events();
    ignored.start().await.expect("start non-cooperative VM");
    wait_for_log(&mut ignored_events, "ignore-cancellation=ready").await;
    let grace = Duration::from_millis(150);
    let before_stop = Instant::now();
    ignored
        .stop(StopMode::Graceful { timeout: grace })
        .await
        .expect("force-stop non-cooperative guest after grace period");
    assert!(
        before_stop.elapsed() >= grace,
        "non-cooperative guest was killed before its grace period"
    );
    let ignored_exit = ignored.wait().await.expect("non-cooperative guest exit");
    assert!(ignored_exit.signal.is_some() || ignored_exit.code.is_some());
    ignored.destroy().await.expect("destroy non-cooperative VM");
    assert!(!runtime_root.join(&ignored_id).exists());
    assert!(!cgroup_root_for_assertion.join(&ignored_id).exists());

    let mut delayed_spec = long_running_spec(rootfs.clone(), "readiness-timeout");
    delayed_spec.command.env.insert(
        String::from("HEPH_TEST_READY_DELAY_MS"),
        String::from("1000"),
    );
    let delayed = timeout_provider
        .provision(delayed_spec)
        .await
        .expect("provision delayed-readiness VM");
    let delayed_id = delayed.id().0.clone();
    assert!(matches!(
        delayed.start().await,
        Err(VmError::Unavailable { resource, .. }) if resource == "guest readiness"
    ));
    delayed
        .destroy()
        .await
        .expect("destroy readiness-timeout VM");
    assert!(!runtime_root.join(&delayed_id).exists());
    assert!(!cgroup_root_for_assertion.join(&delayed_id).exists());
}
