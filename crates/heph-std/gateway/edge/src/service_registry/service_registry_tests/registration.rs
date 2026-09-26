use super::*;

#[test]
fn validates_capacity_identity_readiness_and_fencing() {
    assert!(GatewayServiceRegistry::new(0, 1).is_err());
    assert!(GatewayServiceRegistry::new(1, 32).is_err());
    let registry = GatewayServiceRegistry::new(1, 1).expect("registry");
    let instance = Uuid::new_v4();
    let first = key(instance, 1);
    let second = key(Uuid::new_v4(), 1);
    let vm = vm_for(first);
    let (_states, state_receiver) = state();
    registry
        .register(first, vm.clone(), state_receiver)
        .expect("first registration");
    assert_eq!(
        registry
            .register(
                GatewayServiceInstanceKey {
                    fencing_token: 2,
                    ..first
                },
                vm.clone(),
                state().1,
            )
            .unwrap_err(),
        GatewayServiceRegistryError::Duplicate
    );
    assert_eq!(
        registry.register(first, vm, state().1).unwrap_err(),
        GatewayServiceRegistryError::Duplicate
    );
    let (_second_sender, second_state) = state();
    assert_eq!(
        registry
            .register(second, vm_for(second), second_state)
            .unwrap_err(),
        GatewayServiceRegistryError::CapacityExhausted
    );
    assert_eq!(
        registry.unregister(key(instance, 2)).unwrap_err(),
        GatewayServiceRegistryError::NotFound
    );
    registry.unregister(first).expect("remove first");
    let (_replacement_sender, replacement_state) = state();
    registry
        .register(second, vm_for(second), replacement_state)
        .expect("capacity released");
    drop(registry);
}

#[test]
fn rejects_nonready_and_mismatched_vms() {
    let registry = GatewayServiceRegistry::new(2, 1).expect("registry");
    let instance_key = key(Uuid::new_v4(), 1);
    let (not_ready_sender, not_ready) = watch::channel(ServiceWorkerState::Probing);
    assert_eq!(
        registry
            .register(instance_key, vm_for(instance_key), not_ready)
            .unwrap_err(),
        GatewayServiceRegistryError::NotReady
    );
    not_ready_sender.send_replace(ServiceWorkerState::Ready);
    let wrong = TestVm::new(VmId(String::from("wrong-vm")));
    assert_eq!(
        registry
            .register(instance_key, wrong, state().1)
            .unwrap_err(),
        GatewayServiceRegistryError::VmIdentityMismatch
    );
    let (closed_sender, closed_state) = state();
    drop(closed_sender);
    let closed_key = key(Uuid::new_v4(), 1);
    assert_eq!(
        registry
            .register(closed_key, vm_for(closed_key), closed_state)
            .unwrap_err(),
        GatewayServiceRegistryError::NotReady
    );
    drop(registry);
}
