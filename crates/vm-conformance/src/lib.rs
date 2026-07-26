//! Reusable behavioral conformance tests for [`vm_trait`] providers.
//!
//! Provider crates supply specifications and cleanup assertions through
//! [`ProviderHarness`]. These tests observe only the public VM contract;
//! backend implementation details belong in provider-local suites.

use std::{
    net::{IpAddr, Ipv4Addr},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use tokio::{sync::broadcast, time::timeout};
use vm_trait::{
    NetworkMode, PortForward, PortProtocol, RootFilesystem, StopMode, VmError, VmEvent, VmExit,
    VmId, VmInstance, VmProvider, VmSpec,
};

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);
const NO_EVENT_WINDOW: Duration = Duration::from_millis(25);

/// Optional observable behaviors supported by a provider test fixture.
#[derive(Debug, Clone, Copy)]
pub struct TestCapabilities {
    /// Whether the fixture emits a readiness event after startup.
    pub ready_events: bool,
}

impl Default for TestCapabilities {
    fn default() -> Self {
        Self { ready_events: true }
    }
}

/// Adapter between provider fixtures and the shared contract tests.
pub trait ProviderHarness: Send + Sync {
    /// Returns the provider scoped to this harness.
    fn provider(&self) -> Arc<dyn VmProvider>;

    /// Creates a valid specification that runs until stopped or destroyed.
    fn long_running_spec(&self, id: &str) -> VmSpec;

    /// Creates an optional valid specification with ephemeral user-mode
    /// ingress for providers that implement that trait feature.
    fn ephemeral_ingress_spec(&self, _id: &str) -> Option<VmSpec> {
        None
    }

    /// Describes optional observable behavior available to conformance tests.
    fn capabilities(&self) -> TestCapabilities {
        TestCapabilities::default()
    }

    /// Confirms provider-specific resources were released after destruction.
    fn assert_clean(&self, _id: &VmId) {}
}

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

/// Verifies provider-neutral invalid specifications produce typed errors.
///
/// # Panics
///
/// Panics when an invalid specification is accepted or loses field context.
pub async fn invalid_core_specs_are_typed(harness: &impl ProviderHarness) {
    let provider = harness.provider();
    let mut cases = Vec::new();

    let mut empty_id = harness.long_running_spec("valid-empty-id");
    empty_id.id = VmId(String::new());
    cases.push(("id", empty_id));

    let mut zero_cpu = harness.long_running_spec("invalid-zero-cpu");
    zero_cpu.resources.vcpus = 0;
    cases.push(("resources.vcpus", zero_cpu));

    let mut zero_memory = harness.long_running_spec("invalid-zero-memory");
    zero_memory.resources.memory_mib = 0;
    cases.push(("resources.memory_mib", zero_memory));

    let mut relative_program = harness.long_running_spec("invalid-relative-program");
    relative_program.command.program = String::from("bin/true");
    cases.push(("command.program", relative_program));

    let mut relative_working_dir = harness.long_running_spec("invalid-relative-working-dir");
    relative_working_dir.command.working_dir = Some(PathBuf::from("workspace"));
    cases.push(("command.working_dir", relative_working_dir));

    let mut nul_argument = harness.long_running_spec("invalid-nul-argument");
    nul_argument.command.args.push("bad\0argument".to_owned());
    cases.push(("command.args", nul_argument));

    let mut invalid_environment = harness.long_running_spec("invalid-environment");
    invalid_environment
        .command
        .env
        .insert("BAD=KEY".to_owned(), "value".to_owned());
    cases.push(("command.env", invalid_environment));

    let mut relative_root = harness.long_running_spec("invalid-relative-root");
    relative_root.root = match relative_root.root {
        RootFilesystem::Directory { .. } => RootFilesystem::Directory {
            host_path: PathBuf::from("relative-root"),
        },
        RootFilesystem::Disk {
            format, read_only, ..
        } => RootFilesystem::Disk {
            host_path: PathBuf::from("relative-root"),
            format,
            read_only,
        },
        _ => panic!("test harness returned an unknown root filesystem"),
    };
    cases.push(("root.host_path", relative_root));

    let mut non_loopback = harness.long_running_spec("invalid-non-loopback");
    non_loopback.network = NetworkMode::UserMode {
        ingress: vec![PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            host_port: 8080,
            guest_port: 80,
        }],
    };
    cases.push(("network.ingress[0].bind_addr", non_loopback));

    let mut zero_guest_port = harness.long_running_spec("invalid-zero-guest-port");
    zero_guest_port.network = NetworkMode::UserMode {
        ingress: vec![PortForward {
            protocol: PortProtocol::Tcp,
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            host_port: 8080,
            guest_port: 0,
        }],
    };
    cases.push(("network.ingress[0].guest_port", zero_guest_port));

    let mut duplicate_binding = harness.long_running_spec("invalid-duplicate-binding");
    let duplicate = PortForward {
        protocol: PortProtocol::Tcp,
        bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
        host_port: 18080,
        guest_port: 80,
    };
    duplicate_binding.network = NetworkMode::UserMode {
        ingress: vec![duplicate.clone(), duplicate],
    };
    cases.push(("network.ingress[1]", duplicate_binding));

    for (expected_field, spec) in cases {
        let result = provider.provision(spec).await;
        assert!(
            matches!(
                result,
                Err(VmError::InvalidSpec { ref field, .. })
                    if field.starts_with(expected_field)
            ),
            "expected InvalidSpec for {expected_field}, received {}",
            describe_result(&result)
        );
    }
}

/// Runs every mandatory provider-neutral lifecycle check sequentially.
///
/// # Panics
///
/// Panics when any constituent conformance check fails.
pub async fn lifecycle_suite(harness: &impl ProviderHarness) {
    provision_is_stopped(harness).await;
    concurrent_start_is_shared(harness).await;
    wait_is_shared_and_cached(harness).await;
    destroy_before_start_is_typed(harness).await;
    destroy_running_is_idempotent(harness).await;
    stop_is_idempotent(harness).await;
    identifiers_are_unique_and_reusable(harness).await;
    ephemeral_ingress_is_resolved(harness).await;
    invalid_core_specs_are_typed(harness).await;
}

async fn provision(harness: &impl ProviderHarness, id: &str) -> Arc<dyn VmInstance> {
    harness
        .provider()
        .provision(harness.long_running_spec(id))
        .await
        .expect("provision conformance VM")
}

async fn recv_event(events: &mut broadcast::Receiver<VmEvent>) -> VmEvent {
    timeout(EVENT_TIMEOUT, events.recv())
        .await
        .expect("VM event timeout")
        .expect("VM event channel closed")
}

async fn assert_started(events: &mut broadcast::Receiver<VmEvent>, capabilities: TestCapabilities) {
    assert!(matches!(recv_event(events).await, VmEvent::Started { .. }));
    if capabilities.ready_events {
        let next = recv_event(events).await;
        assert!(
            matches!(next, VmEvent::Ready),
            "expected Ready after Started, received {next:?}"
        );
    }
}

async fn assert_one_exit(events: &mut broadcast::Receiver<VmEvent>, expected: &VmExit) {
    loop {
        if let VmEvent::Exited(exit) = recv_event(events).await {
            assert_eq!(&exit, expected);
            break;
        }
    }
    assert!(
        timeout(NO_EVENT_WINDOW, events.recv()).await.is_err(),
        "received an event after terminal Exited"
    );
}

fn assert_valid_exit(exit: &VmExit) {
    assert!(
        !(exit.code.is_some() && exit.signal.is_some()),
        "exit cannot contain both code and signal"
    );
    assert!(
        exit.code.is_some() || exit.signal.is_some(),
        "exit must contain a code or signal"
    );
}

fn describe_result(result: &Result<Arc<dyn VmInstance>, VmError>) -> String {
    match result {
        Ok(vm) => format!("successful VM {:?}", vm.id()),
        Err(error) => error.to_string(),
    }
}

/// Generates individually named mandatory tests for a harness factory.
///
/// The path must name a zero-argument function returning a type that
/// implements [`ProviderHarness`].
#[macro_export]
macro_rules! provider_conformance_tests {
    ($factory:path) => {
        #[tokio::test]
        async fn conformance_provision_is_stopped() {
            $crate::provision_is_stopped(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_concurrent_start_is_shared() {
            $crate::concurrent_start_is_shared(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_wait_is_shared_and_cached() {
            $crate::wait_is_shared_and_cached(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_destroy_before_start_is_typed() {
            $crate::destroy_before_start_is_typed(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_destroy_running_is_idempotent() {
            $crate::destroy_running_is_idempotent(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_stop_is_idempotent() {
            $crate::stop_is_idempotent(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_identifiers_are_unique_and_reusable() {
            $crate::identifiers_are_unique_and_reusable(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_ephemeral_ingress_is_resolved() {
            $crate::ephemeral_ingress_is_resolved(&$factory()).await;
        }

        #[tokio::test]
        async fn conformance_invalid_core_specs_are_typed() {
            $crate::invalid_core_specs_are_typed(&$factory()).await;
        }
    };
}
