use super::*;

pub(super) fn supervisor_policy(
    instance: ServiceInstancePolicy,
    lease: GatewayServiceLeasePolicy,
) -> GatewayServiceSupervisorPolicy {
    supervisor_policy_with_health(instance, lease, Duration::from_secs(10), 3)
}

pub(super) fn supervisor_policy_with_health(
    instance: ServiceInstancePolicy,
    lease: GatewayServiceLeasePolicy,
    health_interval: Duration,
    health_failure_threshold: u32,
) -> GatewayServiceSupervisorPolicy {
    GatewayServiceSupervisorPolicy {
        lease,
        instance,
        health_interval,
        health_failure_threshold,
        ..GatewayServiceSupervisorPolicy::default()
    }
}

pub(super) fn failure_store() -> Arc<MockFailureStore> {
    Arc::new(MockFailureStore {
        reports: Mutex::new(Vec::new()),
        error: Mutex::new(None),
        started: Notify::new(),
        release: Notify::new(),
        blocked: AtomicBool::new(false),
    })
}

pub(super) fn identity() -> GatewayServiceIdentity {
    GatewayServiceIdentity {
        instance_id: Uuid::from_u128(1),
        gateway_id: Uuid::from_u128(2),
        revision_id: Uuid::from_u128(3),
    }
}

pub(super) fn launch(identity: GatewayServiceIdentity) -> GatewayServiceLaunch {
    let service = GatewayServiceConfig::new(
        8080,
        ServiceProbePath::parse("/ready").expect("readiness"),
        ServiceProbePath::parse("/health").expect("health"),
    )
    .expect("service");
    GatewayServiceLaunch {
        identity,
        service,
        spec: VmSpec {
            id: VmId(format!("gateway-service-{}", identity.instance_id)),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/tmp/coordinator-test-root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 64,
            },
            network: NetworkMode::Disabled,
            command: GuestCommand {
                program: String::from("/service"),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: None,
            },
            runtime_authority: None,
            runtime_git_bridge: None,
            private_http_service: Some(PrivateHttpServiceSpec {
                loopback_port: 8080,
                max_connections: 32,
                connect_timeout: Duration::from_secs(2),
            }),
            labels: BTreeMap::new(),
        },
    }
}

pub(super) fn restore_target(
    identity: GatewayServiceIdentity,
    desired_revision_id: Uuid,
) -> crate::GatewayServiceOwnedTarget {
    let service = GatewayServiceConfig::new(
        8080,
        ServiceProbePath::parse("/ready").expect("readiness"),
        ServiceProbePath::parse("/health").expect("health"),
    )
    .expect("service");
    crate::GatewayServiceOwnedTarget {
        gateway_id: identity.gateway_id,
        lifecycle: String::from("enabled"),
        active_revision_id: Some(identity.revision_id),
        desired_service_revision_id: Some(desired_revision_id),
        revision: crate::GatewayServiceRevisionTarget {
            revision_id: identity.revision_id,
            release_id: Some(Uuid::from_u128(4)),
            release_state: Some(String::from("published")),
            publication_eligible: true,
            service,
        },
    }
}

pub(super) fn lease(
    owner: &GatewayServiceOwner,
    identity: GatewayServiceIdentity,
) -> GatewayServiceInstanceLease {
    let now = ::time::OffsetDateTime::now_utc();
    GatewayServiceInstanceLease {
        identity,
        owner_host_id: owner.host_id.clone(),
        owner_uuid: owner.owner_uuid,
        fencing_token: 1,
        state: GatewayServiceInstanceState::Provisioning,
        vm_id: format!("gateway-service-{}", identity.instance_id),
        lease_expires_at: now + ::time::Duration::seconds(30),
        heartbeat_at: now,
    }
}

pub(super) async fn respond(mut peer: DuplexStream) {
    let mut request = [0_u8; 512];
    let _ = peer.read(&mut request).await;
    peer.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
        .await
        .expect("probe response");
}

pub(super) async fn respond_status(mut peer: DuplexStream, status: u16) {
    let mut request = [0_u8; 512];
    let _ = peer.read(&mut request).await;
    let response =
        format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    peer.write_all(response.as_bytes())
        .await
        .expect("probe response");
}

pub(super) async fn respond_status_notifying(
    mut peer: DuplexStream,
    status: u16,
    replied: Arc<Notify>,
) {
    let mut request = [0_u8; 512];
    let _ = peer.read(&mut request).await;
    let response =
        format!("HTTP/1.1 {status} Test\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
    peer.write_all(response.as_bytes())
        .await
        .expect("probe response");
    replied.notify_one();
}

pub(super) fn mock_ownership() -> Arc<MockOwnership> {
    Arc::new(MockOwnership {
        events: Mutex::new(Vec::new()),
        renewals: AtomicUsize::new(0),
        renewal_activity: None,
        renew_fails: AtomicBool::new(false),
        stopping_fails: AtomicBool::new(false),
        drain_conflict: AtomicBool::new(false),
        drain_blocked: AtomicBool::new(false),
        drain_started: Notify::new(),
        drain_release: Notify::new(),
        promote_fails: AtomicBool::new(false),
        promote_started: Notify::new(),
        promote_release: Notify::new(),
        promote_blocked: AtomicBool::new(false),
    })
}
