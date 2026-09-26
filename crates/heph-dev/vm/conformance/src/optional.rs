use super::{api::ProviderHarness, support::recv_event};
use vm_trait::{StopMode, VmEvent};

/// Verifies provider-allocated ingress ports are resolved in `Started`.
///
/// Providers that do not return an ingress fixture skip this optional feature
/// check while still running every mandatory lifecycle check.
///
/// # Panics
///
/// Panics when ingress startup, allocation, shutdown, or cleanup is incorrect.
pub async fn ephemeral_ingress_is_resolved(harness: &impl ProviderHarness) {
    let Some(spec) = harness.ephemeral_ingress_spec("conformance-ephemeral-ingress") else {
        return;
    };
    let id = spec.id.clone();
    let vm = harness
        .provider()
        .provision(spec)
        .await
        .expect("provision ephemeral-ingress VM");
    let mut events = vm.subscribe_events();
    vm.start().await.expect("start ephemeral-ingress VM");
    let VmEvent::Started { ingress } = recv_event(&mut events).await else {
        panic!("ephemeral-ingress VM did not emit Started first");
    };
    assert!(!ingress.is_empty(), "resolved ingress list is empty");
    assert!(
        ingress.iter().all(|forward| {
            forward.host_port != 0 && forward.guest_port != 0 && forward.bind_addr.is_loopback()
        }),
        "Started contains an unresolved or non-loopback ingress rule: {ingress:?}"
    );
    vm.stop(StopMode::Force)
        .await
        .expect("stop ephemeral-ingress VM");
    vm.destroy().await.expect("destroy ephemeral-ingress VM");
    harness.assert_clean(&id);
}
