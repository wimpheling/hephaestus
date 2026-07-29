//! Deterministic in-memory implementation of the Hephaestus VM contracts.

use async_trait::async_trait;
use std::{
    collections::HashSet,
    net::IpAddr,
    path::Path,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicU16, Ordering},
    },
};
use tokio::sync::{broadcast, watch};
use vm_trait::{
    NetworkMode, PortForward, PortProtocol, RootFilesystem, StopMode, VmError, VmEvent, VmExit,
    VmId, VmInstance, VmProvider, VmSpec,
};

const EVENT_CAPACITY: usize = 64;
const EPHEMERAL_PORT_START: u16 = 49_152;
const EPHEMERAL_PORT_COUNT: u16 = 16_384;

/// A deterministic provider that simulates VMs without starting processes.
///
/// This provider is intended for core lifecycle and orchestration tests. It
/// validates provider-neutral specifications, allocates fake host ports, and
/// implements the same idempotency and exit-caching contract as real
/// providers.
#[derive(Clone)]
pub struct FakeProvider {
    inner: Arc<ProviderInner>,
}

impl FakeProvider {
    /// Creates an empty fake provider.
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(ProviderInner::default()),
        }
    }
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl VmProvider for FakeProvider {
    fn name(&self) -> &'static str {
        "fake"
    }

    async fn provision(&self, spec: VmSpec) -> Result<Arc<dyn VmInstance>, VmError> {
        validate_spec(&spec)?;

        {
            let mut ids = lock(&self.inner.ids);
            if !ids.insert(spec.id.clone()) {
                return Err(VmError::AlreadyExists(spec.id));
            }
        }

        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (terminal, _) = watch::channel(None);
        let instance = FakeInstance {
            id: spec.id.clone(),
            spec,
            provider: Arc::clone(&self.inner),
            state: Mutex::new(InstanceState::Provisioned),
            events,
            terminal,
        };

        Ok(Arc::new(instance))
    }

    async fn cleanup_orphan(&self, id: &VmId) -> Result<(), VmError> {
        if lock(&self.inner.ids).contains(id) {
            return Err(VmError::InvalidState(
                "cannot clean an orphan while a live instance handle is registered",
            ));
        }
        Ok(())
    }
}

#[derive(Default)]
struct ProviderInner {
    ids: Mutex<HashSet<VmId>>,
    ports: Mutex<HashSet<PortBinding>>,
    next_port: AtomicU16,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct PortBinding {
    protocol: PortProtocol,
    bind_addr: IpAddr,
    host_port: u16,
}

struct FakeInstance {
    id: VmId,
    spec: VmSpec,
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

#[async_trait]
impl VmInstance for FakeInstance {
    fn id(&self) -> &VmId {
        &self.id
    }

    async fn start(&self) -> Result<(), VmError> {
        let started = {
            let mut state = lock(&self.state);
            match &*state {
                InstanceState::Provisioned => {
                    let ingress = self.provider.reserve_ingress(&self.spec.network)?;
                    *state = InstanceState::Running {
                        ingress: ingress.clone(),
                    };
                    drop(state);
                    Some(ingress)
                }
                InstanceState::Running { .. } => {
                    drop(state);
                    None
                }
                InstanceState::Exited => {
                    drop(state);
                    return Err(VmError::InvalidState("an exited VM cannot be restarted"));
                }
                InstanceState::Destroyed => {
                    drop(state);
                    return Err(VmError::Destroyed);
                }
            }
        };

        if let Some(ingress) = started {
            send_event(&self.events, VmEvent::Started { ingress });
            send_event(&self.events, VmEvent::Ready);
        }
        Ok(())
    }

    async fn stop(&self, mode: StopMode) -> Result<(), VmError> {
        let exit = match mode {
            StopMode::Graceful { .. } => VmExit {
                code: Some(0),
                signal: None,
            },
            StopMode::Force => VmExit {
                code: None,
                signal: Some(9),
            },
            _ => {
                return Err(VmError::Unsupported {
                    feature: "stop mode".to_owned(),
                    provider: "fake".to_owned(),
                });
            }
        };

        self.finish_running(exit);
        Ok(())
    }

    async fn wait(&self) -> Result<VmExit, VmError> {
        let mut terminal = self.terminal.subscribe();
        loop {
            let current = terminal.borrow_and_update().clone();
            if let Some(result) = current {
                return terminal_result(result);
            }

            terminal
                .changed()
                .await
                .map_err(|source| VmError::Provider {
                    provider: "fake".to_owned(),
                    code: "terminal-channel-closed".to_owned(),
                    source: Box::new(source),
                })?;
        }
    }

    fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
        self.events.subscribe()
    }

    async fn destroy(&self) -> Result<(), VmError> {
        let action = {
            let mut state = lock(&self.state);
            match &*state {
                InstanceState::Provisioned => {
                    *state = InstanceState::Destroyed;
                    DestroyAction::BeforeStart
                }
                InstanceState::Running { ingress } => {
                    let ingress = ingress.clone();
                    let exit = VmExit {
                        code: None,
                        signal: Some(9),
                    };
                    *state = InstanceState::Destroyed;
                    DestroyAction::Running { ingress, exit }
                }
                InstanceState::Exited => {
                    *state = InstanceState::Destroyed;
                    DestroyAction::NoTerminalChange
                }
                InstanceState::Destroyed => DestroyAction::NoTerminalChange,
            }
        };

        match action {
            DestroyAction::BeforeStart => {
                self.terminal.send_replace(Some(Terminal::Destroyed));
            }
            DestroyAction::Running { ingress, exit } => {
                self.provider.release_ingress(&ingress);
                self.terminal
                    .send_replace(Some(Terminal::Exited(exit.clone())));
                send_event(&self.events, VmEvent::Exited(exit));
            }
            DestroyAction::NoTerminalChange => {}
        }

        lock(&self.provider.ids).remove(&self.id);
        Ok(())
    }
}

impl FakeInstance {
    fn finish_running(&self, exit: VmExit) {
        let ingress = {
            let mut state = lock(&self.state);
            if let InstanceState::Running { ingress } = &*state {
                let ingress = ingress.clone();
                *state = InstanceState::Exited;
                drop(state);
                Some(ingress)
            } else {
                drop(state);
                None
            }
        };

        if let Some(ingress) = ingress {
            self.provider.release_ingress(&ingress);
            self.terminal
                .send_replace(Some(Terminal::Exited(exit.clone())));
            send_event(&self.events, VmEvent::Exited(exit));
        }
    }
}

impl ProviderInner {
    fn reserve_ingress(&self, network: &NetworkMode) -> Result<Vec<PortForward>, VmError> {
        let requested = match network {
            NetworkMode::Disabled => return Ok(Vec::new()),
            NetworkMode::UserMode { ingress } => ingress,
            _ => {
                return Err(VmError::Unsupported {
                    feature: "network mode".to_owned(),
                    provider: "fake".to_owned(),
                });
            }
        };

        let mut allocated = Vec::with_capacity(requested.len());
        let mut ports = lock(&self.ports);

        for forward in requested {
            let mut resolved = forward.clone();
            if resolved.host_port == 0 {
                let Some(port) = self.next_available_port(&ports) else {
                    for previous in &allocated {
                        ports.remove(&PortBinding::from(previous));
                    }
                    drop(ports);
                    return Err(VmError::Unavailable {
                        resource: "host port".to_owned(),
                        reason: "the fake ephemeral port range is exhausted".to_owned(),
                    });
                };
                resolved.host_port = port;
            }

            let binding = PortBinding::from(&resolved);
            if !ports.insert(binding.clone()) {
                for previous in &allocated {
                    ports.remove(&PortBinding::from(previous));
                }
                drop(ports);
                return Err(VmError::Unavailable {
                    resource: format!("host port {}", resolved.host_port),
                    reason: "the requested address and port are already reserved".to_owned(),
                });
            }
            allocated.push(resolved);
        }

        drop(ports);
        Ok(allocated)
    }

    fn release_ingress(&self, ingress: &[PortForward]) {
        let mut ports = lock(&self.ports);
        for forward in ingress {
            ports.remove(&PortBinding::from(forward));
        }
    }

    fn next_available_port(&self, ports: &HashSet<PortBinding>) -> Option<u16> {
        for _ in 0..EPHEMERAL_PORT_COUNT {
            let offset = self.next_port.fetch_add(1, Ordering::Relaxed) % EPHEMERAL_PORT_COUNT;
            let candidate = EPHEMERAL_PORT_START + offset;
            let in_use = ports.iter().any(|binding| binding.host_port == candidate);
            if !in_use {
                return Some(candidate);
            }
        }
        None
    }
}

impl From<&PortForward> for PortBinding {
    fn from(forward: &PortForward) -> Self {
        Self {
            protocol: forward.protocol,
            bind_addr: forward.bind_addr,
            host_port: forward.host_port,
        }
    }
}

const fn terminal_result(terminal: Terminal) -> Result<VmExit, VmError> {
    match terminal {
        Terminal::Exited(exit) => Ok(exit),
        Terminal::Destroyed => Err(VmError::Destroyed),
    }
}

fn send_event(events: &broadcast::Sender<VmEvent>, event: VmEvent) {
    drop(events.send(event));
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// Keeping fake validation aligned with the public contract is clearer than
// splitting its small, sequential validation pipeline.
#[allow(clippy::too_many_lines)]
fn validate_spec(spec: &VmSpec) -> Result<(), VmError> {
    validate_nonempty("id", &spec.id.0)?;
    if spec.resources.vcpus == 0 {
        return invalid_spec("resources.vcpus", "must be greater than zero");
    }
    if spec.resources.memory_mib == 0 {
        return invalid_spec("resources.memory_mib", "must be greater than zero");
    }
    validate_absolute("command.program", Path::new(&spec.command.program))?;
    validate_no_nul("command.program", &spec.command.program)?;
    if let Some(working_dir) = &spec.command.working_dir {
        validate_absolute("command.working_dir", working_dir)?;
    }
    for (index, argument) in spec.command.args.iter().enumerate() {
        validate_no_nul(&format!("command.args[{index}]"), argument)?;
    }
    for (key, value) in &spec.command.env {
        validate_no_nul("command.env", key)?;
        validate_no_nul("command.env", value)?;
        if key.contains('=') {
            return invalid_spec("command.env", "keys must not contain '='");
        }
    }

    match &spec.root {
        RootFilesystem::Directory { host_path } | RootFilesystem::Disk { host_path, .. } => {
            validate_absolute("root.host_path", host_path)?;
        }
        _ => {
            return Err(VmError::Unsupported {
                feature: "root filesystem".to_owned(),
                provider: "fake".to_owned(),
            });
        }
    }

    let mut disk_ids = HashSet::new();
    for (index, disk) in spec.disks.iter().enumerate() {
        validate_nonempty(&format!("disks[{index}].id"), &disk.id)?;
        validate_absolute(&format!("disks[{index}].host_path"), &disk.host_path)?;
        if !disk_ids.insert(&disk.id) {
            return invalid_spec(
                &format!("disks[{index}].id"),
                "must be unique within the VM specification",
            );
        }
    }

    let mut mount_tags = HashSet::new();
    for (index, mount) in spec.mounts.iter().enumerate() {
        validate_nonempty(&format!("mounts[{index}].tag"), &mount.tag)?;
        validate_absolute(&format!("mounts[{index}].host_path"), &mount.host_path)?;
        validate_absolute(&format!("mounts[{index}].guest_path"), &mount.guest_path)?;
        if !mount_tags.insert(&mount.tag) {
            return invalid_spec(
                &format!("mounts[{index}].tag"),
                "must be unique within the VM specification",
            );
        }
    }

    match &spec.network {
        NetworkMode::Disabled => {}
        NetworkMode::UserMode { ingress } => {
            let mut fixed_bindings = HashSet::new();
            for (index, forward) in ingress.iter().enumerate() {
                if !forward.bind_addr.is_loopback() {
                    return invalid_spec(
                        &format!("network.ingress[{index}].bind_addr"),
                        "must be a loopback address",
                    );
                }
                if forward.guest_port == 0 {
                    return invalid_spec(
                        &format!("network.ingress[{index}].guest_port"),
                        "must be greater than zero",
                    );
                }
                if !matches!(forward.protocol, PortProtocol::Tcp) {
                    return Err(VmError::Unsupported {
                        feature: "port-forward protocol".to_owned(),
                        provider: "fake".to_owned(),
                    });
                }
                if forward.host_port != 0
                    && !fixed_bindings.insert((
                        forward.protocol,
                        forward.bind_addr,
                        forward.host_port,
                    ))
                {
                    return invalid_spec(
                        &format!("network.ingress[{index}]"),
                        "duplicates an earlier fixed host binding",
                    );
                }
            }
        }
        _ => {
            return Err(VmError::Unsupported {
                feature: "network mode".to_owned(),
                provider: "fake".to_owned(),
            });
        }
    }

    Ok(())
}

fn validate_nonempty(field: &str, value: &str) -> Result<(), VmError> {
    if value.is_empty() {
        return invalid_spec(field, "must not be empty");
    }
    Ok(())
}

fn validate_absolute(field: &str, path: &Path) -> Result<(), VmError> {
    if !path.is_absolute() {
        return invalid_spec(field, "must be an absolute path");
    }
    Ok(())
}

fn validate_no_nul(field: &str, value: &str) -> Result<(), VmError> {
    if value.as_bytes().contains(&0) {
        return invalid_spec(field, "must not contain NUL");
    }
    Ok(())
}

fn invalid_spec<T>(field: &str, reason: &str) -> Result<T, VmError> {
    Err(VmError::InvalidSpec {
        field: field.to_owned(),
        reason: reason.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::FakeProvider;
    use std::{
        collections::BTreeMap,
        fs,
        net::{IpAddr, Ipv4Addr},
        path::PathBuf,
        sync::Arc,
    };
    use tempfile::TempDir;
    use vm_conformance::ProviderHarness;
    use vm_trait::{
        DiskFormat, GuestCommand, NetworkMode, PortForward, PortProtocol, RootFilesystem, VmDisk,
        VmError, VmId, VmMount, VmProvider, VmResources, VmSpec,
    };

    struct FakeHarness {
        provider: Arc<FakeProvider>,
        temp: TempDir,
    }

    impl ProviderHarness for FakeHarness {
        fn provider(&self) -> Arc<dyn VmProvider> {
            self.provider.clone()
        }

        fn long_running_spec(&self, id: &str) -> VmSpec {
            spec(id)
        }

        fn ephemeral_ingress_spec(&self, id: &str) -> Option<VmSpec> {
            Some(spec(id))
        }

        fn caller_owned_spec(&self, id: &str) -> Option<VmSpec> {
            let root = self.temp.path().join("root");
            let disk = self.temp.path().join("agent-state.raw");
            let mount = self.temp.path().join("workspace");
            fs::create_dir_all(&root).unwrap();
            fs::create_dir_all(&mount).unwrap();
            fs::write(&disk, b"persistent-state").unwrap();
            let mut requested = spec(id);
            requested.root = RootFilesystem::Directory { host_path: root };
            requested.disks.push(VmDisk {
                id: String::from("instance-state"),
                host_path: disk,
                format: DiskFormat::Raw,
                read_only: false,
            });
            requested.mounts.push(VmMount {
                tag: String::from("workspace"),
                host_path: mount,
                guest_path: PathBuf::from("/workspace"),
                read_only: false,
            });
            Some(requested)
        }
    }

    fn harness() -> FakeHarness {
        FakeHarness {
            provider: Arc::new(FakeProvider::new()),
            temp: TempDir::new().unwrap(),
        }
    }

    vm_conformance::provider_conformance_tests!(harness);

    fn spec(id: &str) -> VmSpec {
        VmSpec {
            id: VmId(id.to_owned()),
            root: RootFilesystem::Directory {
                host_path: PathBuf::from("/fake/root"),
            },
            disks: Vec::new(),
            mounts: Vec::new(),
            resources: VmResources {
                vcpus: 1,
                memory_mib: 256,
            },
            network: NetworkMode::UserMode {
                ingress: vec![PortForward {
                    protocol: PortProtocol::Tcp,
                    bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
                    host_port: 0,
                    guest_port: 22,
                }],
            },
            command: GuestCommand {
                program: "/bin/true".to_owned(),
                args: Vec::new(),
                env: BTreeMap::new(),
                working_dir: Some(PathBuf::from("/workspace")),
            },
            labels: BTreeMap::new(),
        }
    }

    #[tokio::test]
    async fn invalid_working_directory_is_rejected() {
        let provider = FakeProvider::new();
        let mut invalid = spec("invalid");
        invalid.command.working_dir = Some(PathBuf::from("relative"));

        assert!(matches!(
            provider.provision(invalid).await,
            Err(VmError::InvalidSpec { field, .. }) if field == "command.working_dir"
        ));
    }
}
