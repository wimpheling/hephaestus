use super::{
    support_adapters::{TestFailureStore, TestProvider, TestResolver, TestTargets},
    support_ownership::{TestExactRecovery, TestOwnership},
};
use crate::{
    GatewayServiceBootRecoveryContext, GatewayServiceCleanupDriverPolicy, GatewayServiceIdentity,
    GatewayServiceInstanceLease, GatewayServiceInstanceState, GatewayServiceOwner,
};
use ::time::{Duration as TimeDuration, OffsetDateTime};
use std::sync::{Arc, Mutex, atomic::AtomicUsize};
use uuid::Uuid;

pub(super) fn context_with_owner(
    instances: Vec<GatewayServiceInstanceLease>,
    owner: GatewayServiceOwner,
    ownership: Arc<TestOwnership>,
) -> GatewayServiceBootRecoveryContext {
    let targets = Arc::new(TestTargets {
        inventory: Mutex::new(instances.clone()),
        instances: Mutex::new(instances),
        list_calls: AtomicUsize::new(0),
        hide_after: None,
        cleaned: None,
    });
    context_with_targets_and_provider(
        owner,
        ownership,
        targets,
        Arc::new(TestProvider::standard()),
    )
}

pub(super) fn context_with_targets_and_provider(
    owner: GatewayServiceOwner,
    ownership: Arc<TestOwnership>,
    targets: Arc<TestTargets>,
    provider: Arc<TestProvider>,
) -> GatewayServiceBootRecoveryContext {
    let policy = crate::GatewayServiceSupervisorPolicy::default();
    GatewayServiceBootRecoveryContext {
        owner,
        cleanup_policy: GatewayServiceCleanupDriverPolicy {
            lease: policy.lease,
            database_timeout: policy.instance.probe_timeout,
        },
        shutdown_timeout: policy.instance.shutdown_timeout,
        ownership,
        exact_recovery: Arc::new(TestExactRecovery),
        targets,
        failure_store: Arc::new(TestFailureStore),
        resolver: Arc::new(TestResolver),
        provider,
    }
}

pub(super) fn context(
    instances: Vec<GatewayServiceInstanceLease>,
) -> GatewayServiceBootRecoveryContext {
    context_with_owner(
        instances,
        GatewayServiceOwner::new("boot-test-host", Uuid::new_v4()).expect("owner"),
        Arc::new(TestOwnership::standard()),
    )
}

pub(super) fn inventory_lease() -> GatewayServiceInstanceLease {
    let identity = GatewayServiceIdentity {
        instance_id: Uuid::new_v4(),
        gateway_id: Uuid::new_v4(),
        revision_id: Uuid::new_v4(),
    };
    let now = OffsetDateTime::now_utc();
    GatewayServiceInstanceLease {
        identity,
        owner_host_id: String::from("boot-test-host"),
        owner_uuid: Uuid::new_v4(),
        fencing_token: 1,
        state: GatewayServiceInstanceState::Provisioning,
        vm_id: format!("gateway-service-{}", identity.instance_id),
        lease_expires_at: now + TimeDuration::minutes(1),
        heartbeat_at: now,
    }
}
