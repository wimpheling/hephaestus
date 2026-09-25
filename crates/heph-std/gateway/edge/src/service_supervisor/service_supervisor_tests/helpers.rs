use super::*;
pub(super) fn supervisor_with_policy(
    ownership: Arc<Noop>,
    owner: GatewayServiceOwner,
    policy: GatewayServiceSupervisorPolicy,
) -> GatewayServiceSupervisor {
    let registry = GatewayServiceRegistry::new(8, policy.requests_per_instance).expect("registry");
    GatewayServiceSupervisor::new(GatewayServiceSupervisorContext {
        owner,
        policy,
        ownership,
        failure_store: Arc::new(Noop::default()),
        resolver: Arc::new(Noop::default()),
        provider: Arc::new(Noop::default()),
        targets: Arc::new(Noop::default()),
        registry,
        service_authority: String::from("127.0.0.1:8080"),
    })
    .expect("supervisor")
}

pub(super) fn supervisor_with(ownership: Arc<Noop>) -> GatewayServiceSupervisor {
    supervisor_with_policy(
        ownership,
        GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner"),
        GatewayServiceSupervisorPolicy::default(),
    )
}

pub(super) fn supervisor() -> GatewayServiceSupervisor {
    supervisor_with(Arc::new(Noop::default()))
}

pub(super) fn lease(
    request: GatewayServiceStartupRequest,
    owner: &GatewayServiceOwner,
) -> GatewayServiceInstanceLease {
    let identity = GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: request.gateway_id,
        revision_id: request.revision_id,
    };
    GatewayServiceInstanceLease {
        identity,
        owner_host_id: owner.host_id.clone(),
        owner_uuid: owner.owner_uuid,
        fencing_token: 1,
        state: crate::GatewayServiceInstanceState::Provisioning,
        vm_id: format!("gateway-service-{}", identity.instance_id),
        lease_expires_at: OffsetDateTime::now_utc() + TimeDuration::minutes(1),
        heartbeat_at: OffsetDateTime::now_utc(),
    }
}

#[allow(clippy::too_many_lines)] // Fixture keeps the retryable VM graph explicit.
pub(super) fn ready_supervisor(
    fail_destroy: bool,
) -> (
    GatewayServiceSupervisor,
    Arc<ReadyProvider>,
    Arc<ReadyOwnership>,
    GatewayServiceStartupRequest,
    Arc<AtomicUsize>,
) {
    let gateway_id = Uuid::new_v4();
    let revision_id = Uuid::new_v4();
    let instance_id = Uuid::new_v4();
    let identity = GatewayServiceIdentity {
        instance_id,
        gateway_id,
        revision_id,
    };
    let owner = GatewayServiceOwner::new("ready-host", Uuid::new_v4()).expect("owner");
    let service = GatewayServiceConfig::new(
        8080,
        ServiceProbePath::parse("/ready").expect("readiness"),
        ServiceProbePath::parse("/health").expect("health"),
    )
    .expect("service");
    let launch = crate::GatewayServiceLaunch {
        identity,
        service: service.clone(),
        spec: vm_trait::VmSpec {
            id: VmId(format!("gateway-service-{instance_id}")),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/tmp/supervisor-ready-root"),
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
                connect_timeout: std::time::Duration::from_secs(2),
            }),
            labels: BTreeMap::new(),
        },
    };
    let lease = GatewayServiceInstanceLease {
        identity,
        owner_host_id: owner.host_id.clone(),
        owner_uuid: owner.owner_uuid,
        fencing_token: 1,
        state: crate::GatewayServiceInstanceState::Provisioning,
        vm_id: format!("gateway-service-{instance_id}"),
        lease_expires_at: OffsetDateTime::now_utc() + TimeDuration::minutes(5),
        heartbeat_at: OffsetDateTime::now_utc(),
    };
    let ownership = Arc::new(ReadyOwnership {
        lease: Mutex::new(lease),
        renew_stale: AtomicBool::new(false),
    });
    let fail_destroy = Arc::new(AtomicBool::new(fail_destroy));
    let provider = Arc::new(ReadyProvider {
        fail_destroy: Arc::clone(&fail_destroy),
        destroy_gate: Arc::new(Mutex::new(None)),
        destroy_started: Arc::new(Mutex::new(None)),
        destroy_calls: Arc::new(AtomicUsize::new(0)),
        orphan_cleanup_calls: AtomicUsize::new(0),
        last_vm: Mutex::new(None),
    });
    let target = crate::GatewayServiceOwnedTarget {
        gateway_id,
        lifecycle: String::from("enabled"),
        active_revision_id: None,
        desired_service_revision_id: Some(revision_id),
        revision: crate::GatewayServiceRevisionTarget {
            revision_id,
            release_id: Some(Uuid::new_v4()),
            release_state: Some(String::from("published")),
            publication_eligible: true,
            service,
        },
    };
    let accepted = Arc::new(AtomicUsize::new(0));
    let policy = GatewayServiceSupervisorPolicy::default();
    let registry = GatewayServiceRegistry::new(8, policy.requests_per_instance).expect("registry");
    let supervisor = GatewayServiceSupervisor::new(GatewayServiceSupervisorContext {
        owner,
        policy,
        ownership: Arc::clone(&ownership) as Arc<dyn GatewayServiceOwnership>,
        failure_store: Arc::new(ReadyFailureStore),
        resolver: Arc::new(ReadyResolver { launch }),
        provider: Arc::clone(&provider) as Arc<dyn VmProvider>,
        targets: Arc::new(ReadyTargets {
            target,
            ownership: Arc::clone(&ownership),
            accepted: Arc::clone(&accepted),
        }),
        registry,
        service_authority: String::from("127.0.0.1:8080"),
    })
    .expect("supervisor");
    (
        supervisor,
        provider,
        ownership,
        GatewayServiceStartupRequest {
            gateway_id,
            revision_id,
            intent: GatewayServiceStartupIntent::ActivateDesired,
        },
        accepted,
    )
}

pub(super) fn request(gateway_id: Uuid, revision_id: Uuid) -> GatewayServiceStartupRequest {
    GatewayServiceStartupRequest {
        gateway_id,
        revision_id,
        intent: GatewayServiceStartupIntent::ActivateDesired,
    }
}

pub(super) fn insert_known_claim_record(
    supervisor: &mut GatewayServiceSupervisor,
    request: GatewayServiceStartupRequest,
    lease: &GatewayServiceInstanceLease,
) -> Uuid {
    let token = supervisor
        .capacity
        .lock()
        .expect("capacity")
        .reserve(request.gateway_id, request.revision_id)
        .expect("capacity reservation");
    let id = Uuid::new_v4();
    let (status, _) = watch::channel(GatewayServiceSupervisorJobStatus::Uncertain);
    supervisor.records.insert(
        id,
        JobRecord {
            request,
            token,
            cancellation: CancellationToken::new(),
            status,
            capacity_retained: true,
            completion: Some(JobTerminal {
                lease: Some(lease.clone()),
                claim_uncertain: false,
                coordinator_failure: None,
                claim_cleanup_reason: Some(GatewayServiceCoordinatorFailureReason::Cancelled),
            }),
            cleanup_retry: None,
            cleanup_in_flight: false,
            claim_resolution_in_flight: false,
        },
    );
    id
}

pub(super) async fn wait_until_ready(
    supervisor: &mut GatewayServiceSupervisor,
    handle: &GatewayServiceStartupHandle,
) {
    let mut observed = handle.subscribe();
    let poll = supervisor.poll();
    tokio::pin!(poll);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            tokio::select! {
                changed = observed.changed() => {
                    changed.expect("status channel");
                    if *observed.borrow() == GatewayServiceSupervisorJobStatus::Ready {
                        break;
                    }
                }
                event = &mut poll => panic!("startup ended before readiness: {event:?}"),
            }
        }
    })
    .await
    .expect("bounded readiness");
}
