use crate::{
    validation::{PROVIDER_NAME, PreparedForward, PreparedNetwork},
    worker::WorkerConfiguration,
};
use std::{
    fs, io,
    net::{IpAddr, SocketAddr, TcpListener},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::Instant,
};

pub struct PasstProcess {
    child: Child,
    socket_path: PathBuf,
    pid_path: PathBuf,
    pub ingress: Vec<PreparedForward>,
}

impl PasstProcess {
    pub fn start(
        config: &WorkerConfiguration,
        network: &PreparedNetwork,
        runtime_dir: &Path,
    ) -> Result<Option<Self>, WorkerNetworkError> {
        let PreparedNetwork::UserMode { ingress } = network else {
            return Ok(None);
        };

        let (reservations, resolved) = reserve_ports(ingress)?;
        let socket_path = runtime_dir.join("passt.sock");
        let pid_path = runtime_dir.join("passt.pid");
        let log_path = runtime_dir.join("passt.log");

        let mut command = Command::new(&config.passt_binary);
        command
            .arg("--foreground")
            .arg("--one-off")
            .arg("--quiet")
            .arg("--socket")
            .arg(&socket_path)
            .arg("--pid")
            .arg(&pid_path)
            .arg("--log-file")
            .arg(&log_path)
            .arg("--log-size")
            .arg("1048576")
            .arg("--runas")
            .arg(format!("{}:{}", config.service_uid, config.service_gid))
            .arg("--udp-ports")
            .arg("none");

        if resolved.is_empty() {
            command.arg("--tcp-ports").arg("none");
        } else {
            for forward in &resolved {
                command.arg("--tcp-ports").arg(format_forward(forward));
            }
        }
        command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        drop(reservations);
        let mut child = command.spawn().map_err(WorkerNetworkError::Spawn)?;
        let deadline = Instant::now() + config.startup_timeout;
        loop {
            if socket_path.exists() {
                break;
            }
            if let Some(status) = child.try_wait().map_err(WorkerNetworkError::Wait)? {
                return Err(WorkerNetworkError::Exited(status.code()));
            }
            if Instant::now() >= deadline {
                let _kill_result = child.kill();
                let _wait_result = child.wait();
                return Err(WorkerNetworkError::Timeout);
            }
            thread::sleep(std::time::Duration::from_millis(10));
        }

        tracing::info!(
            provider = PROVIDER_NAME,
            passt_pid = child.id(),
            socket = %socket_path.display(),
            forwards = resolved.len(),
            "passt egress backend ready"
        );
        Ok(Some(Self {
            child,
            socket_path,
            pid_path,
            ingress: resolved,
        }))
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for PasstProcess {
    fn drop(&mut self) {
        let _kill_result = self.child.kill();
        let _wait_result = self.child.wait();
        let _socket_result = fs::remove_file(&self.socket_path);
        let _pid_result = fs::remove_file(&self.pid_path);
    }
}

fn reserve_ports(
    ingress: &[PreparedForward],
) -> Result<(Vec<TcpListener>, Vec<PreparedForward>), WorkerNetworkError> {
    let mut listeners = Vec::with_capacity(ingress.len());
    let mut resolved = Vec::with_capacity(ingress.len());
    for forward in ingress {
        let listener = TcpListener::bind(SocketAddr::new(forward.bind_addr, forward.host_port))
            .map_err(WorkerNetworkError::Reserve)?;
        let host_port = listener
            .local_addr()
            .map_err(WorkerNetworkError::Reserve)?
            .port();
        listeners.push(listener);
        resolved.push(PreparedForward {
            bind_addr: forward.bind_addr,
            host_port,
            guest_port: forward.guest_port,
        });
    }
    Ok((listeners, resolved))
}

fn format_forward(forward: &PreparedForward) -> String {
    match forward.bind_addr {
        IpAddr::V4(address) => {
            format!("{address}/{}:{}", forward.host_port, forward.guest_port)
        }
        IpAddr::V6(address) => {
            format!("[{address}]/{}:{}", forward.host_port, forward.guest_port)
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkerNetworkError {
    #[error("failed to reserve ingress port: {0}")]
    Reserve(io::Error),
    #[error("failed to spawn passt: {0}")]
    Spawn(io::Error),
    #[error("failed to inspect passt: {0}")]
    Wait(io::Error),
    #[error("passt exited before accepting the VMM connection (status {0:?})")]
    Exited(Option<i32>),
    #[error("passt did not create its socket before the startup timeout")]
    Timeout,
}

#[cfg(test)]
mod tests {
    use super::{format_forward, reserve_ports};
    use crate::validation::PreparedForward;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn zero_port_is_resolved_and_reserved() {
        let requested = [PreparedForward {
            bind_addr: IpAddr::V4(Ipv4Addr::LOCALHOST),
            host_port: 0,
            guest_port: 22,
        }];
        let (listeners, resolved) = reserve_ports(&requested).unwrap();
        assert_eq!(listeners.len(), 1);
        assert_ne!(resolved[0].host_port, 0);
        assert_eq!(
            format_forward(&resolved[0]),
            format!("127.0.0.1/{}:22", resolved[0].host_port)
        );
    }
}
