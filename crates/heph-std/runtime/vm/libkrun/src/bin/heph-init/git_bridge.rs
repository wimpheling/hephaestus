//! Guest-local loopback proxy for the internal runtime-Git vsock stream.

use std::{
    io::{self, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    os::fd::{AsRawFd, OwnedFd, RawFd},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use vm_libkrun::protocol::{RUNTIME_GIT_VSOCK_PORT, RuntimeGitBridgeMessage};

use super::vsock;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const ACCEPT_POLL: Duration = Duration::from_millis(25);
const MAX_CONNECTIONS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    pub repository_id: uuid::Uuid,
    pub loopback_port: u16,
}

impl TryFrom<&RuntimeGitBridgeMessage> for Config {
    type Error = io::Error;

    fn try_from(message: &RuntimeGitBridgeMessage) -> Result<Self, Self::Error> {
        if message.repository_id.is_nil() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime Git repository ID must not be nil",
            ));
        }
        if !(1024..=u16::MAX).contains(&message.loopback_port) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "runtime Git loopback port must be between 1024 and 65535",
            ));
        }
        Ok(Self {
            repository_id: message.repository_id,
            loopback_port: message.loopback_port,
        })
    }
}

pub struct Supervisor {
    cancelled: Arc<AtomicBool>,
    active: Arc<Mutex<Vec<OwnedFd>>>,
    listener_thread: Option<JoinHandle<()>>,
    connection_threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Supervisor {
    pub fn start(config: Config) -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", config.loopback_port))?;
        listener.set_nonblocking(true)?;
        let cancelled = Arc::new(AtomicBool::new(false));
        let active = Arc::new(Mutex::new(Vec::<OwnedFd>::new()));
        let connection_threads = Arc::new(Mutex::new(Vec::<JoinHandle<()>>::new()));
        let listener_cancelled = Arc::clone(&cancelled);
        let listener_active = Arc::clone(&active);
        let listener_threads = Arc::clone(&connection_threads);
        let listener_thread = thread::Builder::new()
            .name(String::from("heph-runtime-git-listener"))
            .spawn(move || {
                while !listener_cancelled.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((local, _)) => {
                            let full = {
                                let mut threads = listener_threads
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                                let mut index = 0;
                                while index < threads.len() {
                                    if threads[index].is_finished() {
                                        let handle = threads.swap_remove(index);
                                        let _ = handle.join();
                                    } else {
                                        index += 1;
                                    }
                                }
                                threads.len() >= MAX_CONNECTIONS
                            };
                            if full {
                                let _ = local.shutdown(Shutdown::Both);
                                continue;
                            }
                            let cancel = Arc::clone(&listener_cancelled);
                            let active = Arc::clone(&listener_active);
                            let handle = thread::spawn(move || {
                                let _ = bridge(&local, &cancel, &active);
                            });
                            listener_threads
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner)
                                .push(handle);
                        }
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(ACCEPT_POLL);
                        }
                        Err(_) => break,
                    }
                }
            })?;
        Ok(Self {
            cancelled,
            active,
            listener_thread: Some(listener_thread),
            connection_threads,
        })
    }

    pub fn stop(mut self) {
        self.cancelled.store(true, Ordering::Release);
        let active = self
            .active
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for fd in active.iter() {
            shutdown_fd(fd.as_raw_fd());
        }
        drop(active);
        if let Some(handle) = self.listener_thread.take() {
            let _ = handle.join();
        }
        let handles = std::mem::take(
            &mut *self
                .connection_threads
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        for handle in handles {
            let _ = handle.join();
        }
    }
}

fn bridge(
    local: &TcpStream,
    cancelled: &AtomicBool,
    active: &Mutex<Vec<OwnedFd>>,
) -> io::Result<()> {
    local.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    local.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let host = vsock::connect_host_with_cancel(RUNTIME_GIT_VSOCK_PORT, CONNECT_TIMEOUT, cancelled)?;
    let local_reader = local.try_clone()?;
    let local_writer = local.try_clone()?;
    let host_reader = host.try_clone()?;
    let host_writer = host.try_clone()?;
    let active_local: OwnedFd = local.try_clone()?.into();
    let active_host: OwnedFd = host.try_clone()?.into();
    let local_fd = local.as_raw_fd();
    let host_fd = host.as_raw_fd();
    register(active, active_local);
    register(active, active_host);
    let left = thread::spawn(move || forward(local_reader, host_writer));
    let right = thread::spawn(move || forward(host_reader, local_writer));
    let _ = left.join();
    let _ = right.join();
    shutdown_fd(local_fd);
    shutdown_fd(host_fd);
    unregister(active, local_fd);
    unregister(active, host_fd);
    Ok(())
}

fn forward<R: Read, W: Write + AsRawFd>(mut reader: R, mut writer: W) {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                shutdown_write_fd(writer.as_raw_fd());
                return;
            }
            Ok(read) if writer.write_all(&buffer[..read]).is_err() => {
                shutdown_fd(writer.as_raw_fd());
                return;
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return,
        }
    }
}

fn register(active: &Mutex<Vec<OwnedFd>>, fd: OwnedFd) {
    active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(fd);
}

fn unregister(active: &Mutex<Vec<OwnedFd>>, fd: RawFd) {
    active
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .retain(|candidate| candidate.as_raw_fd() != fd);
}

#[allow(unsafe_code)]
fn shutdown_write_fd(fd: RawFd) {
    // SAFETY: descriptors are borrowed from live stream owners and shutdown
    // only their write half to preserve a response after request EOF.
    let _ = unsafe { libc::shutdown(fd, libc::SHUT_WR) };
}

#[allow(unsafe_code)]
fn shutdown_fd(fd: RawFd) {
    // SAFETY: descriptors are borrowed from live stream owners and shutdown
    // only interrupts the bridge's blocking I/O.
    let _ = unsafe { libc::shutdown(fd, libc::SHUT_RDWR) };
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read, Write},
        net::{Shutdown, TcpListener, TcpStream},
        thread,
    };

    use super::{Config, forward};
    use vm_libkrun::protocol::RuntimeGitBridgeMessage;

    #[test]
    fn config_rejects_nil_repository() {
        let message = RuntimeGitBridgeMessage {
            repository_id: uuid::Uuid::nil(),
            loopback_port: 19_100,
        };
        assert!(Config::try_from(&message).is_err());
    }

    #[test]
    fn config_accepts_an_unprivileged_route() {
        let message = RuntimeGitBridgeMessage {
            repository_id: uuid::Uuid::new_v4(),
            loopback_port: 19_100,
        };
        let config = Config::try_from(&message).expect("valid runtime Git route");
        assert_eq!(config.loopback_port, 19_100);
        assert_eq!(config.repository_id, message.repository_id);
    }

    #[test]
    fn forwarding_preserves_response_after_request_half_close() {
        fn pair() -> (TcpStream, TcpStream) {
            let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind test listener");
            let address = listener.local_addr().expect("test listener address");
            let client = TcpStream::connect(address).expect("connect test listener");
            let (server, _) = listener.accept().expect("accept test client");
            (client, server)
        }

        let (client, proxy_local) = pair();
        let (proxy_host, server) = pair();
        let local_to_host = thread::spawn({
            let local_reader = proxy_local.try_clone().expect("clone local reader");
            let host_writer = proxy_host.try_clone().expect("clone host writer");
            move || forward(local_reader, host_writer)
        });
        let host_to_local = thread::spawn(move || forward(proxy_host, proxy_local));

        let mut client = client;
        client.write_all(b"request").expect("write request");
        client
            .shutdown(Shutdown::Write)
            .expect("half-close request");
        let mut request = Vec::new();
        let mut server = server;
        server
            .read_to_end(&mut request)
            .expect("read forwarded request");
        assert_eq!(request, b"request");
        server.write_all(b"response").expect("write response");
        server
            .shutdown(Shutdown::Write)
            .expect("half-close response");
        let mut response = Vec::new();
        client
            .read_to_end(&mut response)
            .expect("read forwarded response");
        assert_eq!(response, b"response");
        local_to_host.join().expect("request forward thread");
        host_to_local.join().expect("response forward thread");
    }
}
