use super::*;
pub(super) fn fixture() -> (
    GatewayServiceOwner,
    GatewayServiceInstanceLease,
    GatewayServiceCleanupDriverPolicy,
) {
    let owner = GatewayServiceOwner::new("test-host", Uuid::new_v4()).expect("owner");
    let identity = GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let now = OffsetDateTime::now_utc();
    let lease = GatewayServiceInstanceLease {
        identity,
        owner_host_id: owner.host_id.clone(),
        owner_uuid: owner.owner_uuid,
        fencing_token: 1,
        state: GatewayServiceInstanceState::Stopping,
        vm_id: format!("gateway-service-{}", identity.instance_id),
        lease_expires_at: now + ::time::Duration::seconds(10),
        heartbeat_at: now,
    };
    let policy = GatewayServiceCleanupDriverPolicy {
        lease: GatewayServiceLeasePolicy {
            lease_duration: Duration::from_millis(300),
            renewal_interval: Duration::from_millis(30),
        },
        database_timeout: Duration::from_secs(1),
    };
    (owner, lease, policy)
}

pub(super) fn driver_parts(
    owner: &GatewayServiceOwner,
    policy: GatewayServiceCleanupDriverPolicy,
    provider: Arc<TestProvider>,
    ownership: Arc<TestOwnership>,
    failures: Arc<TestFailures>,
    targets: Arc<TestTargets>,
) -> GatewayServiceCleanupDriver {
    GatewayServiceCleanupDriver::new(
        ownership,
        failures,
        targets,
        provider,
        Arc::new(TestResolver),
        owner.clone(),
        policy,
    )
    .expect("driver")
}
