use super::*;

fn id(value: u128) -> Uuid {
    Uuid::from_u128(value)
}

fn manager() -> GatewayServiceCapacity {
    GatewayServiceCapacity::new(GatewayServiceSupervisorPolicy::default()).expect("default policy")
}

#[test]
fn defaults_are_validated_and_bounded() {
    let policy = GatewayServiceSupervisorPolicy::default();
    policy.validate().expect("default policy is valid");
    assert_eq!(policy.serving_gateway_capacity, 8);
    assert_eq!(policy.replacement_capacity, 2);
    assert_eq!(policy.requests_per_instance, 16);
    assert_eq!(policy.lease.lease_duration, Duration::from_secs(30));
    assert_eq!(policy.lease.renewal_interval, Duration::from_secs(5));
    assert_eq!(policy.instance.startup_timeout, Duration::from_secs(120));
    assert_eq!(policy.instance.probe_interval, Duration::from_millis(500));
    assert_eq!(policy.instance.probe_timeout, Duration::from_secs(2));
    assert_eq!(policy.instance.shutdown_timeout, Duration::from_secs(10));
    assert_eq!(policy.health_failure_threshold, 3);
    assert_eq!(policy.drain_timeout, Duration::from_secs(30));
}

#[test]
fn invalid_policy_and_identity_fail_closed() {
    let policy = GatewayServiceSupervisorPolicy {
        requests_per_instance: MAX_SERVICE_REQUEST_CAPACITY + 1,
        ..GatewayServiceSupervisorPolicy::default()
    };
    assert_eq!(
        GatewayServiceCapacity::new(policy).unwrap_err(),
        GatewayServiceCapacityError::InvalidPolicy
    );
    let policy = GatewayServiceSupervisorPolicy {
        health_failure_threshold: 4,
        health_interval: Duration::ZERO,
        ..GatewayServiceSupervisorPolicy::default()
    };
    assert_eq!(
        GatewayServiceCapacity::new(policy).unwrap_err(),
        GatewayServiceCapacityError::InvalidPolicy
    );
    let policy = GatewayServiceSupervisorPolicy {
        replacement_capacity: 0,
        ..GatewayServiceSupervisorPolicy::default()
    };
    assert_eq!(
        GatewayServiceCapacity::new(policy).unwrap_err(),
        GatewayServiceCapacityError::InvalidPolicy
    );
    let policy = GatewayServiceSupervisorPolicy {
        max_revisions_per_gateway: 1,
        ..GatewayServiceSupervisorPolicy::default()
    };
    assert_eq!(
        GatewayServiceCapacity::new(policy).unwrap_err(),
        GatewayServiceCapacityError::InvalidPolicy
    );
    let mut capacity = manager();
    assert_eq!(
        capacity.reserve(Uuid::nil(), id(1)).unwrap_err(),
        GatewayServiceCapacityError::InvalidPolicy
    );
}

#[test]
fn full_serving_pool_allows_replacement_but_not_new_gateway() {
    let mut capacity = manager();
    for gateway in 1..=8 {
        let token = capacity
            .reserve(id(gateway), id(gateway + 100))
            .expect("serving slot");
        capacity.finish_startup(token).expect("serving instance");
    }
    assert_eq!(capacity.snapshot().serving_gateways, 8);
    let first_replacement = capacity.reserve(id(1), id(201)).expect("replacement slot");
    capacity
        .finish_startup(first_replacement)
        .expect("replacement serving");
    assert_eq!(capacity.snapshot().live_instances, 9);
    assert_eq!(
        capacity.reserve(id(9), id(109)).unwrap_err(),
        GatewayServiceCapacityError::ServingCapacityExhausted
    );
    let second_replacement = capacity
        .reserve(id(2), id(202))
        .expect("second replacement slot");
    capacity
        .finish_startup(second_replacement)
        .expect("second replacement serving");
    assert_eq!(capacity.snapshot().live_instances, 10);
    assert_eq!(
        capacity.reserve(id(3), id(203)).unwrap_err(),
        GatewayServiceCapacityError::TotalCapacityExhausted
    );
}

#[test]
fn revision_and_startup_bounds_are_independent() {
    let mut capacity = manager();
    let first = capacity.reserve(id(1), id(101)).expect("first startup");
    let second = capacity.reserve(id(2), id(102)).expect("second startup");
    assert_eq!(
        capacity.reserve(id(3), id(103)).unwrap_err(),
        GatewayServiceCapacityError::StartupCapacityExhausted
    );
    capacity.finish_startup(first).expect("first ready");
    let third = capacity
        .reserve(id(3), id(103))
        .expect("startup allowance released");
    assert_eq!(
        capacity.reserve(id(1), id(101)).unwrap_err(),
        GatewayServiceCapacityError::DuplicateRevision
    );
    capacity.finish_startup(second).expect("second ready");
    capacity.finish_startup(third).expect("third ready");
    assert_eq!(capacity.snapshot().starting_instances, 0);
}

#[test]
fn draining_blocks_replacement_until_exact_cleanup() {
    let mut capacity = manager();
    let old = capacity.reserve(id(1), id(101)).expect("old");
    capacity.finish_startup(old).expect("old ready");
    let candidate = capacity.reserve(id(1), id(102)).expect("candidate");
    assert_eq!(
        capacity.reserve(id(1), id(103)).unwrap_err(),
        GatewayServiceCapacityError::RevisionCapacityExhausted
    );
    capacity.complete(old).expect("old cleanup");
    let next = capacity.reserve(id(1), id(103)).expect("next replacement");
    capacity.complete(candidate).expect("candidate cleanup");
    capacity.complete(next).expect("next cleanup");
    assert_eq!(capacity.snapshot().live_instances, 0);
}

#[test]
fn stale_or_dropped_tokens_cannot_release_a_new_reservation() {
    let mut capacity = manager();
    let old = capacity.reserve(id(1), id(101)).expect("old");
    let stale = old;
    capacity.complete(old).expect("old cleanup");
    let replacement = capacity.reserve(id(1), id(102)).expect("replacement");
    assert_eq!(
        capacity.complete(stale).unwrap_err(),
        GatewayServiceCapacityError::UnknownReservation
    );
    assert_eq!(capacity.snapshot().live_instances, 1);
    assert_eq!(capacity.finish_startup(replacement), Ok(()));
    assert_eq!(
        capacity.finish_startup(replacement).unwrap_err(),
        GatewayServiceCapacityError::StartupAlreadyComplete
    );
    capacity.complete(replacement).expect("replacement cleanup");
}

#[test]
fn counts_do_not_leak_between_gateways() {
    let mut capacity = manager();
    let first = capacity.reserve(id(1), id(101)).expect("first");
    capacity.finish_startup(first).expect("first ready");
    let second = capacity.reserve(id(2), id(101)).expect("different gateway");
    assert_eq!(capacity.snapshot().serving_gateways, 2);
    assert_eq!(capacity.snapshot().live_instances, 2);
    capacity.complete(first).expect("first cleanup");
    capacity.complete(second).expect("second cleanup");
}

#[test]
fn reservation_drop_does_not_implicitly_release_capacity() {
    let mut capacity = manager();
    let _token = capacity.reserve(id(1), id(101)).expect("reservation");
    assert_eq!(capacity.snapshot().live_instances, 1);
}
