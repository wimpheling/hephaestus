use super::*;

#[tokio::test]
async fn cancellation_during_resolution_settles_then_cleans() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    resolver.block.store(true, Ordering::Relaxed);
    let provider = TestProvider::new(TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    ));
    let (handle, preparation) = new_service_preparation(
        resolver.clone(),
        provider.clone(),
        GatewayServiceLaunchRequest { identity },
    );
    let task = tokio::spawn(preparation.run());
    resolver.resolve_started.notified().await;
    drop(handle);
    resolver.release_resolve.notify_one();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(failure.reason, ServicePreparationFailureReason::Cancelled);
    assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
    assert_eq!(provider.cleanup_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn cancellation_during_provision_destroys_late_vm_before_materialization() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    let vm = TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    );
    let provider = TestProvider::new(vm.clone());
    provider.block.store(true, Ordering::Relaxed);
    let (handle, preparation) = new_service_preparation(
        resolver.clone(),
        provider.clone(),
        GatewayServiceLaunchRequest { identity },
    );
    let task = tokio::spawn(preparation.run());
    resolver.resolve_started.notified().await;
    provider.provision_started.notified().await;
    handle.cancel();
    provider.release_provision.notify_one();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(failure.reason, ServicePreparationFailureReason::Cancelled);
    assert!(failure.vm.is_none());
    assert_eq!(vm.destroy_calls.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
    assert!(resolver.cleanup_observed_destroy.load(Ordering::Relaxed));
}

#[tokio::test]
async fn failed_destroy_retains_vm_and_materialization() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    let vm = TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    );
    vm.destroy_fail.store(true, Ordering::Relaxed);
    let provider = TestProvider::new(vm.clone());
    provider.block.store(true, Ordering::Relaxed);
    let (handle, preparation) = new_service_preparation(
        resolver.clone(),
        provider.clone(),
        GatewayServiceLaunchRequest { identity },
    );
    let task = tokio::spawn(preparation.run());
    provider.provision_started.notified().await;
    handle.cancel();
    provider.release_provision.notify_one();
    let failure = task.await.unwrap().unwrap_err();
    assert_eq!(
        failure.reason,
        ServicePreparationFailureReason::CleanupIncomplete
    );
    assert!(failure.vm.is_some());
    assert!(failure.materialization_owned);
    assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn provider_error_requires_orphan_confirmation_before_materialization_cleanup() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    let provider = TestProvider::new(TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    ));
    provider.fail.store(true, Ordering::Relaxed);
    provider.cleanup_fail.store(true, Ordering::Relaxed);
    let (_handle, preparation) = new_service_preparation(
        resolver.clone(),
        provider.clone(),
        GatewayServiceLaunchRequest { identity },
    );
    let failure = preparation.run().await.unwrap_err();
    assert_eq!(
        failure.reason,
        ServicePreparationFailureReason::CleanupIncomplete
    );
    assert!(failure.materialization_owned);
    assert_eq!(provider.cleanup_calls.load(Ordering::Relaxed), 1);
    assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn normal_preparation_returns_stopped_vm() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    let vm = TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    );
    let provider = TestProvider::new(vm.clone());
    let (_handle, preparation) =
        new_service_preparation(resolver, provider, GatewayServiceLaunchRequest { identity });
    let prepared = preparation.run().await.unwrap();
    assert_eq!(prepared.vm.id(), &vm.id);
    assert_eq!(vm.start_calls.load(Ordering::Relaxed), 0);
    assert_eq!(vm.destroy_calls.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn resolver_error_still_attempts_exact_materialization_cleanup() {
    let identity = identity();
    let resolver = TestResolver::new(launch(identity));
    resolver.fail.store(true, Ordering::Relaxed);
    let vm = TestVm::new(
        VmId(format!("gateway-service-{}", identity.instance_id)),
        resolver.destroyed_before_cleanup.clone(),
    );
    let provider = TestProvider::new(vm);
    let (_handle, preparation) = new_service_preparation(
        resolver.clone(),
        provider,
        GatewayServiceLaunchRequest { identity },
    );
    let failure = preparation.run().await.unwrap_err();
    assert_eq!(failure.reason, ServicePreparationFailureReason::Resolution);
    assert!(!failure.materialization_owned);
    assert_eq!(resolver.cleanup_calls.load(Ordering::Relaxed), 1);
}
