//! Deterministic in-memory implementation of the Hephaestus VM contracts.

use async_trait::async_trait;
use std::{
    collections::HashSet,
    net::IpAddr,
    sync::{Arc, Mutex, MutexGuard, atomic::AtomicU16},
};
use tokio::sync::{broadcast, watch};
use vm_trait::{
    PortForward, PortProtocol, PrivateHttpRequest, PrivateHttpResponse, VmError, VmEvent, VmExit,
    VmId,
};

const EVENT_CAPACITY: usize = 64;
const EPHEMERAL_PORT_START: u16 = 49_152;
const EPHEMERAL_PORT_COUNT: u16 = 16_384;

#[path = "fake/api.rs"]
mod api;
#[path = "fake/ingress.rs"]
mod ingress;
#[path = "fake/lifecycle.rs"]
mod lifecycle;
#[path = "fake/validation.rs"]
mod validation;

#[cfg(test)]
#[path = "fake/tests.rs"]
mod tests;

pub use api::FakeProvider;

/// Deterministic private guest handler used by [`FakeProvider`] tests.
#[async_trait]
pub trait PrivateHttpResponder: Send + Sync {
    /// Handles one complete private HTTP request.
    async fn invoke(&self, request: PrivateHttpRequest) -> Result<PrivateHttpResponse, VmError>;
}

struct ProviderInner {
    ids: Mutex<HashSet<VmId>>,
    ports: Mutex<HashSet<PortBinding>>,
    next_port: AtomicU16,
    private_http_responder: Mutex<Option<Arc<dyn PrivateHttpResponder>>>,
}

impl Default for ProviderInner {
    fn default() -> Self {
        Self {
            ids: Mutex::new(HashSet::new()),
            ports: Mutex::new(HashSet::new()),
            next_port: AtomicU16::new(0),
            private_http_responder: Mutex::new(None),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PortBinding {
    protocol: PortProtocol,
    bind_addr: IpAddr,
    host_port: u16,
}

struct FakeInstance {
    id: VmId,
    spec: vm_trait::VmSpec,
    provider: Arc<ProviderInner>,
    state: Mutex<InstanceState>,
    events: broadcast::Sender<VmEvent>,
    terminal: watch::Sender<Option<Terminal>>,
}

enum InstanceState {
    Provisioned,
    Running { ingress: Vec<PortForward> },
    Exited,
    Destroyed,
}

#[derive(Clone)]
enum Terminal {
    Exited(VmExit),
    Destroyed,
}

enum DestroyAction {
    BeforeStart,
    Running {
        ingress: Vec<PortForward>,
        exit: VmExit,
    },
    NoTerminalChange,
}

fn send_event(events: &broadcast::Sender<VmEvent>, event: VmEvent) {
    drop(events.send(event));
}

const fn terminal_result(terminal: Terminal) -> Result<VmExit, VmError> {
    match terminal {
        Terminal::Exited(exit) => Ok(exit),
        Terminal::Destroyed => Err(VmError::Destroyed),
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
