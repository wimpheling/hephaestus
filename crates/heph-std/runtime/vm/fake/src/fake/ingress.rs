use super::{EPHEMERAL_PORT_COUNT, EPHEMERAL_PORT_START, PortBinding, ProviderInner, lock};
use std::{collections::HashSet, sync::atomic::Ordering};
use vm_trait::{NetworkMode, PortForward, VmError};

impl ProviderInner {
    pub(super) fn reserve_ingress(
        &self,
        network: &NetworkMode,
    ) -> Result<Vec<PortForward>, VmError> {
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

    pub(super) fn release_ingress(&self, ingress: &[PortForward]) {
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
