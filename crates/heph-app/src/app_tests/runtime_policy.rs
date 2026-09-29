use crate::{
    RuntimePolicy, StoredNetworkAccess, deterministic_update_hook_run_id, guest_environment,
    validate_runtime_policy,
};
use heph_run::RunKind;
use heph_runtime::{VmError, VmResources};
use uuid::Uuid;

fn policy() -> RuntimePolicy {
    RuntimePolicy {
        version: String::from("test/v2"),
        max_vcpus: 2,
        max_memory_mib: 1_024,
        allow_broker_only: true,
        allow_egress: false,
    }
}

#[test]
fn recovered_update_hook_attempts_have_distinct_stable_ids() {
    let update_id = Uuid::new_v4();
    let first = deterministic_update_hook_run_id(update_id, 0);
    let retry = deterministic_update_hook_run_id(update_id, 1);
    assert_eq!(first, deterministic_update_hook_run_id(update_id, 0));
    assert_ne!(first, retry);
    assert_eq!(first.as_uuid().get_version_num(), 8);
    assert_eq!(retry.as_uuid().get_version_num(), 8);
}

#[test]
fn current_platform_policy_accepts_an_unchanged_allowed_contract() {
    validate_runtime_policy(
        &policy(),
        &VmResources {
            vcpus: 2,
            memory_mib: 1_024,
        },
        StoredNetworkAccess::BrokerOnly,
    )
    .expect("contract remains allowed");
}
#[test]
fn current_platform_policy_rejects_stored_resources_over_the_new_ceiling() {
    let error = validate_runtime_policy(
        &policy(),
        &VmResources {
            vcpus: 3,
            memory_mib: 1_024,
        },
        StoredNetworkAccess::Disabled,
    )
    .expect_err("contract exceeds the current ceiling");

    assert!(matches!(
        error,
        VmError::InvalidSpec { ref field, .. }
            if field == "effective_runtime_policy.resources"
    ));
}

#[test]
fn current_platform_policy_rejects_network_access_disabled_since_resolution() {
    let error = validate_runtime_policy(
        &policy(),
        &VmResources {
            vcpus: 1,
            memory_mib: 512,
        },
        StoredNetworkAccess::Egress,
    )
    .expect_err("egress is no longer allowed");

    assert!(matches!(
        error,
        VmError::InvalidSpec { ref field, .. }
            if field == "effective_runtime_policy.network"
    ));
}

#[test]
fn update_guest_receives_the_exact_stable_update_identity() {
    let update_id = Uuid::new_v4();
    let expected = update_id.to_string();
    let environment =
        guest_environment(RunKind::Update, Some(update_id)).expect("update environment");

    assert_eq!(
        environment.get("HEPHAESTUS_UPDATE_ID").map(String::as_str),
        Some(expected.as_str())
    );
    assert!(
        guest_environment(RunKind::Update, None).is_err(),
        "an update hook must fail closed when its durable identity is absent"
    );
    assert!(
        guest_environment(RunKind::Normal, Some(update_id))
            .expect("normal environment")
            .is_empty(),
        "normal agents must not receive an unrelated update identity"
    );
}
