use super::{
    api::ProviderHarness,
    paths::snapshots,
    support::{NO_EVENT_WINDOW, assert_one_exit, assert_started, assert_valid_exit, provision},
};
use std::{sync::Arc, time::Duration};
use tokio::time::timeout;
use vm_trait::{StopMode, VmError};

/// Verifies that provisioning produces a stopped, event-free instance.
///
/// # Panics
///
/// Panics when provisioning or destruction violates the VM contract.
pub async fn provision_is_stopped(harness: &impl ProviderHarness) {
    let provider = harness.provider();
    assert!(
        !provider.name().is_empty(),
        "provider name must not be empty"
    );
    let vm = provision(harness, "conformance-provision").await;
    let id = vm.id().clone();
    let mut events = vm.subscribe_events();
    assert!(
        timeout(NO_EVENT_WINDOW, events.recv()).await.is_err(),
        "provision emitted an event before start"
    );
    vm.destroy().await.expect("destroy provisioned VM");
    assert!(matches!(vm.wait().await, Err(VmError::Destroyed)));
    harness.assert_clean(&id);
}

/// Verifies that concurrent starts share one lifecycle transition.
///
/// # Panics
///
/// Panics when startup, events, exit caching, or cleanup is incorrect.
pub async fn concurrent_start_is_shared(harness: &impl ProviderHarness) {
    let vm = provision(harness, "conformance-concurrent-start").await;
    let id = vm.id().clone();
    let mut events = vm.subscribe_events();
    let first = Arc::clone(&vm);
    let second = Arc::clone(&vm);
    let (first, second) = tokio::join!(first.start(), second.start());
    first.expect("first concurrent start");
    second.expect("second concurrent start");

    assert_started(&mut events, harness.capabilities()).await;
    vm.stop(StopMode::Graceful {
        timeout: Duration::from_secs(2),
    })
    .await
    .expect("stop concurrently-started VM");
    let expected = vm.wait().await.expect("cache stopped VM exit");
    assert_valid_exit(&expected);
    assert_one_exit(&mut events, &expected).await;
    vm.destroy().await.expect("destroy concurrently-started VM");
    harness.assert_clean(&id);
}

/// Verifies that concurrent and later waiters receive one cached exit.
///
/// # Panics
///
/// Panics when waiters disagree or cleanup is incomplete.
pub async fn wait_is_shared_and_cached(harness: &impl ProviderHarness) {
    let vm = provision(harness, "conformance-wait").await;
    let id = vm.id().clone();
    vm.start().await.expect("start wait test VM");
    let first = Arc::clone(&vm);
    let second = Arc::clone(&vm);
    let (first, second, stopped) = tokio::join!(
        first.wait(),
        second.wait(),
        vm.stop(StopMode::Graceful {
            timeout: Duration::from_secs(2),
        })
    );
    stopped.expect("stop wait test VM");
    let first = first.expect("first waiter");
    let second = second.expect("second waiter");
    assert_eq!(first, second);
    assert_eq!(vm.wait().await.expect("cached waiter"), first);
    vm.destroy().await.expect("destroy wait test VM");
    assert_eq!(vm.wait().await.expect("exit survives destroy"), first);
    harness.assert_clean(&id);
}

/// Verifies destroy-before-start behavior and repeated destruction.
///
/// # Panics
///
/// Panics when destruction is not idempotent or does not wake waiters.
pub async fn destroy_before_start_is_typed(harness: &impl ProviderHarness) {
    let vm = provision(harness, "conformance-destroy-before-start").await;
    let id = vm.id().clone();
    let waiter = Arc::clone(&vm);
    let (wait_result, destroyed) = tokio::join!(waiter.wait(), vm.destroy());
    destroyed.expect("destroy VM before start");
    assert!(matches!(wait_result, Err(VmError::Destroyed)));
    assert!(matches!(vm.wait().await, Err(VmError::Destroyed)));
    assert!(matches!(vm.start().await, Err(VmError::Destroyed)));
    vm.destroy().await.expect("repeat destroy before start");
    harness.assert_clean(&id);
}

/// Verifies that destroying a running VM yields one durable exit.
///
/// # Panics
///
/// Panics when force cleanup, terminal events, or cached exit behavior fails.
pub async fn destroy_running_is_idempotent(harness: &impl ProviderHarness) {
    let vm = provision(harness, "conformance-destroy-running").await;
    let id = vm.id().clone();
    let mut events = vm.subscribe_events();
    vm.start().await.expect("start destroy-running VM");
    assert_started(&mut events, harness.capabilities()).await;
    vm.destroy().await.expect("destroy running VM");
    vm.destroy().await.expect("repeat running VM destroy");
    let exit = vm.wait().await.expect("destroyed running VM exit");
    assert_valid_exit(&exit);
    assert_one_exit(&mut events, &exit).await;
    assert_eq!(vm.wait().await.expect("cached destroyed exit"), exit);
    harness.assert_clean(&id);
}

/// Verifies that destruction preserves every caller-owned host path.
///
/// # Panics
///
/// Panics when a provider deletes or modifies a supplied file backing path.
pub async fn caller_owned_paths_survive_destroy(harness: &impl ProviderHarness) {
    let Some(spec) = harness.caller_owned_spec("conformance-caller-owned") else {
        return;
    };
    let snapshots = snapshots(&spec);
    let vm = harness
        .provider()
        .provision(spec)
        .await
        .expect("provision caller-owned VM");
    vm.destroy().await.expect("destroy caller-owned VM");
    for snapshot in snapshots {
        snapshot.assert_unchanged();
    }
}

/// Verifies repeated stop calls and stop-before-start behavior.
///
/// # Panics
///
/// Panics when idempotent stop behavior changes the documented lifecycle.
pub async fn stop_is_idempotent(harness: &impl ProviderHarness) {
    let vm = provision(harness, "conformance-stop-before-start").await;
    let id = vm.id().clone();
    vm.stop(StopMode::Force).await.expect("stop provisioned VM");
    vm.start().await.expect("start after provisioned stop");
    vm.stop(StopMode::Force).await.expect("force stop VM");
    vm.stop(StopMode::Force)
        .await
        .expect("repeat force stop VM");
    let exit = vm.wait().await.expect("force-stop exit");
    assert_valid_exit(&exit);
    assert!(matches!(vm.start().await, Err(VmError::InvalidState(_))));
    vm.destroy().await.expect("destroy force-stopped VM");
    harness.assert_clean(&id);
}

/// Verifies identifier collision and reuse after destruction.
///
/// # Panics
///
/// Panics when duplicate identifiers are accepted or remain reserved.
pub async fn identifiers_are_unique_and_reusable(harness: &impl ProviderHarness) {
    let provider = harness.provider();
    let spec = harness.long_running_spec("conformance-reusable-id");
    let id = spec.id.clone();
    let vm = provider
        .provision(spec)
        .await
        .expect("provision reusable ID");
    assert!(matches!(
        provider
            .provision(harness.long_running_spec(&id.0))
            .await,
        Err(VmError::AlreadyExists(existing)) if existing == id
    ));
    vm.destroy().await.expect("release reusable ID");
    harness.assert_clean(&id);
    let replacement = provider
        .provision(harness.long_running_spec(&id.0))
        .await
        .expect("reuse destroyed ID");
    replacement.destroy().await.expect("destroy replacement");
    harness.assert_clean(&id);
}
