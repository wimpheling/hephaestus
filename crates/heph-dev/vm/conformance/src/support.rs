use super::api::{ProviderHarness, TestCapabilities};
use std::sync::Arc;
use std::time::Duration;
use tokio::{sync::broadcast, time::timeout};
use vm_trait::{VmError, VmEvent, VmExit, VmInstance};

const EVENT_TIMEOUT: Duration = Duration::from_secs(5);
pub const NO_EVENT_WINDOW: Duration = Duration::from_millis(25);

pub async fn provision(harness: &impl ProviderHarness, id: &str) -> Arc<dyn VmInstance> {
    harness
        .provider()
        .provision(harness.long_running_spec(id))
        .await
        .expect("provision conformance VM")
}

pub async fn recv_event(events: &mut broadcast::Receiver<VmEvent>) -> VmEvent {
    timeout(EVENT_TIMEOUT, events.recv())
        .await
        .expect("VM event timeout")
        .expect("VM event channel closed")
}

pub async fn assert_started(
    events: &mut broadcast::Receiver<VmEvent>,
    capabilities: TestCapabilities,
) {
    assert!(matches!(recv_event(events).await, VmEvent::Started { .. }));
    if capabilities.ready_events {
        let next = recv_event(events).await;
        assert!(
            matches!(next, VmEvent::Ready),
            "expected Ready after Started, received {next:?}"
        );
    }
}

pub async fn assert_one_exit(events: &mut broadcast::Receiver<VmEvent>, expected: &VmExit) {
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

pub fn assert_valid_exit(exit: &VmExit) {
    assert!(
        !(exit.code.is_some() && exit.signal.is_some()),
        "exit cannot contain both code and signal"
    );
    assert!(
        exit.code.is_some() || exit.signal.is_some(),
        "exit must contain a code or signal"
    );
}

pub fn describe_result(result: &Result<Arc<dyn VmInstance>, VmError>) -> String {
    match result {
        Ok(vm) => format!("successful VM {:?}", vm.id()),
        Err(error) => error.to_string(),
    }
}
