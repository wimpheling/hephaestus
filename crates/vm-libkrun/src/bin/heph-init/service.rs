//! Guest-side private service loopback and vsock bridge.

use std::{
    collections::HashMap,
    fs::File,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    os::fd::{AsRawFd, OwnedFd, RawFd},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use vm_libkrun::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC,
    PRIVATE_SERVICE_HANDSHAKE_VERSION, PRIVATE_SERVICE_VSOCK_PORT, PrivateHttpServiceMessage,
    PrivateServiceConnectionMessage,
};

use super::vsock;

pub const SERVICE_HOST: &str = "127.0.0.1";
pub const SERVICE_HOST_ENV: &str = "HEPH_SERVICE_HOST";
pub const SERVICE_PORT_ENV: &str = "HEPH_SERVICE_PORT";
const MAX_SERVICE_TIMEOUT: Duration = Duration::from_secs(30);
const SERVICE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
const JOIN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ServiceConfig {
    pub loopback_port: u16,
    pub max_connections: usize,
    pub connect_timeout: Duration,
}

impl TryFrom<&PrivateHttpServiceMessage> for ServiceConfig {
    type Error = io::Error;

    fn try_from(message: &PrivateHttpServiceMessage) -> Result<Self, Self::Error> {
        if !(1024..=u16::MAX).contains(&message.loopback_port) {
            return Err(invalid("loopback port must be between 1024 and 65535"));
        }
        if !(1..=64).contains(&message.max_connections) {
            return Err(invalid("service connection limit must be between 1 and 64"));
        }
        let timeout = Duration::from_millis(message.connect_timeout_ms);
        if timeout.is_zero() || timeout > MAX_SERVICE_TIMEOUT {
            return Err(invalid(
                "service connect timeout must be between 1ms and 30s",
            ));
        }
        Ok(Self {
            loopback_port: message.loopback_port,
            max_connections: message.max_connections as usize,
            connect_timeout: timeout,
        })
    }
}

#[derive(Debug)]
struct State {
    closing: bool,
    next_id: u64,
    active: HashMap<u64, Vec<Arc<OwnedFd>>>,
    pending: usize,
}

#[derive(Clone)]
pub struct ServiceSupervisor {
    inner: Arc<Inner>,
    handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

struct Inner {
    config: ServiceConfig,
    state: Mutex<State>,
    slots: Condvar,
    cancelled: AtomicBool,
}

impl ServiceSupervisor {
    #[must_use]
    pub fn new(config: ServiceConfig) -> Self {
        Self {
            inner: Arc::new(Inner {
                config,
                state: Mutex::new(State {
                    closing: false,
                    next_id: 0,
                    active: HashMap::new(),
                    pending: 0,
                }),
                slots: Condvar::new(),
                cancelled: AtomicBool::new(false),
            }),
            handles: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub fn open(&self, connection: PrivateServiceConnectionMessage) {
        let deadline = std::time::Instant::now() + self.inner.config.connect_timeout;
        let mut handles = lock(&self.handles);
        reap_finished_locked(&mut handles);
        let Some(id) = self.reserve_pending() else {
            return;
        };
        let supervisor = self.clone();
        let handle = thread::spawn(move || {
            let _ = supervisor.bridge(id, &connection, deadline);
            supervisor.release(id);
        });
        handles.push(handle);
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::Release);
        let fds = {
            let mut state = lock(&self.inner.state);
            state.closing = true;
            state
                .active
                .values()
                .flat_map(|values| values.iter().cloned())
                .collect::<Vec<_>>()
        };
        self.inner.slots.notify_all();
        for fd in fds {
            shutdown_fd(fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }

    pub fn join_all(&self) {
        self.cancel();
        let deadline = std::time::Instant::now() + JOIN_TIMEOUT;
        loop {
            self.reap_finished();
            if lock(&self.handles).is_empty() {
                return;
            }
            if std::time::Instant::now() >= deadline {
                self.cancel();
                let handles = std::mem::take(&mut *lock(&self.handles));
                for handle in handles {
                    let _ = handle.join();
                }
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn reap_finished(&self) {
        let mut handles = lock(&self.handles);
        reap_finished_locked(&mut handles);
    }

    fn reserve_pending(&self) -> Option<u64> {
        let mut state = lock(&self.inner.state);
        if state.closing || state.pending >= self.inner.config.max_connections {
            return None;
        }
        let id = state.next_id;
        state.next_id = state.next_id.wrapping_add(1);
        state.pending += 1;
        drop(state);
        Some(id)
    }

    fn acquire_active(&self, id: u64, deadline: std::time::Instant) -> io::Result<()> {
        let mut state = lock(&self.inner.state);
        loop {
            if std::time::Instant::now() >= deadline {
                state.pending = state.pending.saturating_sub(1);
                drop(state);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service connection admission timed out",
                ));
            }
            if state.closing {
                state.pending = state.pending.saturating_sub(1);
                drop(state);
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "service connection cancelled",
                ));
            }
            if state.active.len() < self.inner.config.max_connections {
                state.pending = state.pending.saturating_sub(1);
                state.active.insert(id, Vec::new());
                drop(state);
                return Ok(());
            }
            let wait = match remaining(deadline) {
                Ok(wait) => wait,
                Err(error) => {
                    state.pending = state.pending.saturating_sub(1);
                    drop(state);
                    return Err(error);
                }
            };
            let (next, result) = self
                .inner
                .slots
                .wait_timeout(state, wait)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if result.timed_out() && state.active.len() >= self.inner.config.max_connections {
                state.pending = state.pending.saturating_sub(1);
                drop(state);
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service connection admission timed out",
                ));
            }
        }
    }

    fn register_fds(&self, id: u64, fds: Vec<Arc<OwnedFd>>) -> bool {
        let mut state = lock(&self.inner.state);
        if state.closing {
            return false;
        }
        state.active.get_mut(&id).is_some_and(|active| {
            *active = fds;
            true
        })
    }

    fn release(&self, id: u64) {
        lock(&self.inner.state).active.remove(&id);
        self.inner.slots.notify_one();
    }

    fn bridge(
        &self,
        id: u64,
        connection: &PrivateServiceConnectionMessage,
        deadline: std::time::Instant,
    ) -> io::Result<()> {
        self.acquire_active(id, deadline)?;
        let mut host = vsock::connect_host_with_cancel(
            PRIVATE_SERVICE_VSOCK_PORT,
            remaining(deadline)?,
            &self.inner.cancelled,
        )?;
        let host_guard: Arc<OwnedFd> = Arc::new(host.try_clone()?.into());
        if !self.register_fds(id, vec![host_guard.clone()]) {
            return Ok(());
        }
        if self.inner.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        let address = SocketAddr::from(([127, 0, 0, 1], self.inner.config.loopback_port));
        let local = connect_loopback(address, deadline, &self.inner.cancelled)?;
        set_socket_timeout(host.as_raw_fd(), libc::SO_RCVTIMEO, SERVICE_IDLE_TIMEOUT)?;
        set_socket_timeout(host.as_raw_fd(), libc::SO_SNDTIMEO, remaining(deadline)?)?;
        local.set_read_timeout(Some(SERVICE_IDLE_TIMEOUT))?;
        local.set_write_timeout(Some(SERVICE_IDLE_TIMEOUT))?;
        if self.inner.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        write_handshake(&mut host, connection)?;
        set_socket_timeout(host.as_raw_fd(), libc::SO_SNDTIMEO, SERVICE_IDLE_TIMEOUT)?;
        let local_guard: OwnedFd = local.try_clone()?.into();
        let local_fd = local.as_raw_fd();
        let host_fd = host.as_raw_fd();
        if !self.register_fds(id, vec![Arc::new(local_guard), host_guard]) {
            shutdown_fd(local_fd, libc::SHUT_RDWR);
            shutdown_fd(host_fd, libc::SHUT_RDWR);
            return Ok(());
        }
        bridge_streams(&local, &host);
        Ok(())
    }
}

fn reap_finished_locked(handles: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let handle = handles.swap_remove(index);
            let _ = handle.join();
        } else {
            index += 1;
        }
    }
}

fn bridge_streams(local: &TcpStream, host: &File) {
    let Ok(local_reader) = local.try_clone() else {
        return;
    };
    let Ok(host_reader) = host.try_clone() else {
        return;
    };
    let Ok(local_writer) = local.try_clone() else {
        return;
    };
    let Ok(host_writer) = host.try_clone() else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(2);
    let left_sender = sender.clone();
    let left = thread::spawn(move || {
        let _ = left_sender.send(forward(local_reader, host_writer));
    });
    let right = thread::spawn(move || {
        let _ = sender.send(forward(host_reader, local_writer));
    });
    let first = receiver.recv().unwrap_or(ForwardResult::Error);
    if first == ForwardResult::Error {
        shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
        shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
    }
    let second = receiver.recv().unwrap_or(ForwardResult::Error);
    if second == ForwardResult::Error {
        shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
        shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
    }
    let _ = left.join();
    let _ = right.join();
    shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
    shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ForwardResult {
    Eof,
    Error,
}

fn forward<R: Read, W: Write + AsRawFd>(mut reader: R, mut writer: W) -> ForwardResult {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                shutdown_fd(writer.as_raw_fd(), libc::SHUT_WR);
                return ForwardResult::Eof;
            }
            Ok(read) if writer.write_all(&buffer[..read]).is_err() => return ForwardResult::Error,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return ForwardResult::Error,
        }
    }
}

fn remaining(deadline: std::time::Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "service connection timed out"))
}

fn connect_loopback(
    address: SocketAddr,
    deadline: std::time::Instant,
    cancelled: &AtomicBool,
) -> io::Result<TcpStream> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "service connection cancelled",
            ));
        }
        let timeout = remaining(deadline)?.min(Duration::from_millis(50));
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error),
        }
    }
}

fn write_handshake(
    host: &mut File,
    connection: &PrivateServiceConnectionMessage,
) -> io::Result<()> {
    let mut frame = [0_u8; 8 + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES];
    frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()]
        .copy_from_slice(&PRIVATE_SERVICE_HANDSHAKE_MAGIC);
    frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] = PRIVATE_SERVICE_HANDSHAKE_VERSION;
    let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
    frame[id_start..id_start + 16].copy_from_slice(connection.connection_id.as_bytes());
    frame[id_start + 16..].copy_from_slice(connection.challenge.as_bytes());
    host.write_all(&frame)
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[allow(unsafe_code)]
fn shutdown_fd(fd: RawFd, how: libc::c_int) {
    // SAFETY: descriptors are borrowed from live socket owners in the active
    // registry; shutdown only changes their connection state.
    let _ = unsafe { libc::shutdown(fd, how) };
}

#[allow(unsafe_code)]
fn set_socket_timeout(fd: RawFd, option: libc::c_int, timeout: Duration) -> io::Result<()> {
    let seconds = timeout.as_secs().min(i32::MAX as u64);
    let value = libc::timeval {
        tv_sec: seconds.try_into().expect("timeout seconds fit timeval"),
        tv_usec: timeout.subsec_micros().into(),
    };
    // SAFETY: `value` is a valid timeval and the descriptor belongs to the
    // live AF_VSOCK connection.
    let result = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&raw const value).cast(),
            libc::socklen_t::try_from(std::mem::size_of_val(&value))
                .expect("timeval size fits socklen_t"),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[allow(unsafe_code)]
pub fn bring_up_loopback() -> io::Result<()> {
    let socket = unsafe { libc::socket(libc::AF_INET, libc::SOCK_DGRAM | libc::SOCK_CLOEXEC, 0) };
    if socket < 0 {
        return Err(io::Error::last_os_error());
    }
    let result = bring_up_loopback_on_socket(socket);
    // SAFETY: `socket` was returned by `socket` above and remains owned here.
    unsafe { libc::close(socket) };
    result
}

#[allow(unsafe_code)]
fn bring_up_loopback_on_socket(socket: RawFd) -> io::Result<()> {
    let mut request: libc::ifreq = unsafe { std::mem::zeroed() };
    let name = b"lo\0";
    // SAFETY: `ifr_name` is the fixed-size interface-name field and `name` is
    // shorter than it.
    unsafe {
        std::ptr::copy_nonoverlapping(
            name.as_ptr().cast::<libc::c_char>(),
            request.ifr_name.as_mut_ptr(),
            name.len(),
        );
    }
    // SAFETY: `request` is a valid ifreq buffer for these ioctl operations.
    if unsafe { libc::ioctl(socket, get_flags_ioctl(), &mut request) } < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: Linux exposes ifru_flags as the active member after SIOCGIFFLAGS.
    let flags = unsafe { request.ifr_ifru.ifru_flags };
    let updated = loopback_flags_up(flags);
    if updated != flags {
        // SAFETY: Linux consumes the same ifreq layout for SIOCSIFFLAGS.
        request.ifr_ifru.ifru_flags = updated;
        if unsafe { libc::ioctl(socket, set_flags_ioctl(), &request) } < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(target_env = "musl")]
fn get_flags_ioctl() -> libc::c_int {
    libc::SIOCGIFFLAGS
        .try_into()
        .expect("SIOCGIFFLAGS fits musl ioctl request")
}

#[cfg(not(target_env = "musl"))]
const fn get_flags_ioctl() -> libc::c_ulong {
    libc::SIOCGIFFLAGS
}

#[cfg(target_env = "musl")]
fn set_flags_ioctl() -> libc::c_int {
    libc::SIOCSIFFLAGS
        .try_into()
        .expect("SIOCSIFFLAGS fits musl ioctl request")
}

#[cfg(not(target_env = "musl"))]
const fn set_flags_ioctl() -> libc::c_ulong {
    libc::SIOCSIFFLAGS
}

fn loopback_flags_up(flags: libc::c_short) -> libc::c_short {
    flags | libc::c_short::try_from(libc::IFF_UP).expect("IFF_UP fits c_short")
}

#[cfg(test)]
mod tests {
    use super::{ServiceConfig, ServiceSupervisor, bridge_streams, forward, loopback_flags_up};
    use std::{
        fs::File,
        io::{self, Read, Write},
        net::{TcpListener, TcpStream},
        os::fd::OwnedFd,
        os::unix::net::UnixStream,
        sync::mpsc,
        thread,
        time::Duration,
    };
    use uuid::Uuid;
    use vm_libkrun::protocol::{
        PRIVATE_SERVICE_CHALLENGE_BYTES, PrivateHttpServiceMessage, PrivateServiceChallenge,
        PrivateServiceConnectionMessage,
    };

    #[test]
    fn config_rejects_invalid_values() {
        let invalid_port = PrivateHttpServiceMessage {
            loopback_port: 80,
            max_connections: 1,
            connect_timeout_ms: 1,
        };
        assert!(ServiceConfig::try_from(&invalid_port).is_err());
        let invalid_limit = PrivateHttpServiceMessage {
            loopback_port: 8080,
            max_connections: 65,
            connect_timeout_ms: 1,
        };
        assert!(ServiceConfig::try_from(&invalid_limit).is_err());
        let invalid_timeout = PrivateHttpServiceMessage {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout_ms: 30_001,
        };
        assert!(ServiceConfig::try_from(&invalid_timeout).is_err());
    }

    #[test]
    fn loopback_flag_update_preserves_existing_flags() {
        let iff_up = i16::try_from(libc::IFF_UP).expect("IFF_UP fits i16");
        assert_eq!(loopback_flags_up(0x10), 0x10 | iff_up);
        assert_eq!(loopback_flags_up(iff_up), iff_up);
    }

    #[test]
    fn forwarding_preserves_half_close() {
        let (mut source_peer, source) = UnixStream::pair().unwrap();
        let (destination, mut destination_peer) = UnixStream::pair().unwrap();
        let bridge = thread::spawn(move || forward(source, destination));
        source_peer.write_all(b"request").unwrap();
        source_peer.shutdown(std::net::Shutdown::Write).unwrap();
        let mut request = Vec::new();
        destination_peer.read_to_end(&mut request).unwrap();
        assert_eq!(request, b"request");
        bridge.join().unwrap();
    }

    #[test]
    fn host_close_wakes_idle_loopback_reader() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let (done_sender, done_receiver) = mpsc::channel();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = [0_u8; 1];
            let read = stream.read(&mut request).unwrap();
            assert_eq!(read, 0, "host close should FIN the loopback write side");
            drop(stream);
            done_sender.send(()).unwrap();
        });
        let local = TcpStream::connect(address).unwrap();
        let (host_peer, host_side) = UnixStream::pair().unwrap();
        let host_fd: OwnedFd = host_side.into();
        let host = File::from(host_fd);
        let bridge = thread::spawn(move || bridge_streams(&local, &host));
        drop(host_peer);
        done_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("idle loopback reader did not receive EOF");
        bridge.join().unwrap();
        server.join().unwrap();
    }

    #[test]
    fn admission_is_bounded_and_cancelled() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_millis(1),
        });
        let first = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        assert!(supervisor.reserve_pending().is_some());
        assert!(supervisor.reserve_pending().is_none());
        supervisor.cancel();
        assert!(supervisor.reserve_pending().is_none());
    }

    #[test]
    fn pending_admission_waits_for_released_active_slot() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_secs(1),
        });
        let first = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        let second = supervisor.reserve_pending().unwrap();
        let waiter_supervisor = supervisor.clone();
        let waiter = thread::spawn(move || {
            waiter_supervisor
                .acquire_active(second, std::time::Instant::now() + Duration::from_secs(1))
        });
        thread::sleep(Duration::from_millis(25));
        assert!(!waiter.is_finished());
        supervisor.release(first);
        waiter.join().unwrap().unwrap();
        supervisor.release(second);
        supervisor.cancel();
    }

    #[test]
    fn expired_admission_does_not_take_a_free_slot() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_secs(1),
        });
        let id = supervisor.reserve_pending().unwrap();
        let result = supervisor.acquire_active(
            id,
            std::time::Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("test instant remains representable"),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        let next = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(next, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        supervisor.release(next);
        supervisor.cancel();
    }

    #[test]
    fn pending_admission_honors_setup_deadline() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_secs(1),
        });
        let first = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        let second = supervisor.reserve_pending().unwrap();
        let result = supervisor.acquire_active(
            second,
            std::time::Instant::now() + Duration::from_millis(25),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        supervisor.release(first);
        supervisor.cancel();
    }

    #[test]
    fn pending_admission_cancels_without_sleep_polling() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_secs(1),
        });
        let first = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(first, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        let second = supervisor.reserve_pending().unwrap();
        let waiter_supervisor = supervisor.clone();
        let waiter = thread::spawn(move || {
            waiter_supervisor
                .acquire_active(second, std::time::Instant::now() + Duration::from_secs(1))
        });
        thread::sleep(Duration::from_millis(25));
        supervisor.cancel();
        assert_eq!(
            waiter.join().unwrap().unwrap_err().kind(),
            io::ErrorKind::Interrupted
        );
        supervisor.release(first);
    }

    #[test]
    fn failed_connect_releases_admission() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_millis(1),
        });
        supervisor.open(PrivateServiceConnectionMessage {
            connection_id: Uuid::new_v4(),
            challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
        });
        thread::sleep(Duration::from_millis(250));
        let next = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(next, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        supervisor.release(next);
        supervisor.cancel();
        supervisor.join_all();
    }

    #[test]
    fn cancellation_closes_owned_descriptor_snapshot() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_secs(1),
        });
        let (mut peer, socket) = UnixStream::pair().unwrap();
        let id = supervisor.reserve_pending().unwrap();
        supervisor
            .acquire_active(id, std::time::Instant::now() + Duration::from_secs(1))
            .unwrap();
        let owned: OwnedFd = socket.try_clone().unwrap().into();
        assert!(supervisor.register_fds(id, vec![std::sync::Arc::new(owned)]));
        drop(socket);
        supervisor.cancel();
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
        supervisor.release(id);
    }

    #[test]
    fn completed_handle_registry_is_reaped_under_churn() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_millis(1),
        });
        for _ in 0..32 {
            supervisor.open(PrivateServiceConnectionMessage {
                connection_id: Uuid::new_v4(),
                challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
            });
            thread::sleep(Duration::from_millis(5));
        }
        supervisor.reap_finished();
        assert!(supervisor.inner.state.lock().unwrap().active.is_empty());
        assert!(supervisor.handles.lock().unwrap().len() <= 1);
        supervisor.cancel();
        supervisor.join_all();
    }

    #[test]
    fn open_keeps_existing_live_handle_registered() {
        let supervisor = ServiceSupervisor::new(ServiceConfig {
            loopback_port: 8080,
            max_connections: 1,
            connect_timeout: Duration::from_millis(1),
        });
        let blocker = thread::spawn(|| thread::sleep(Duration::from_millis(100)));
        supervisor.handles.lock().unwrap().push(blocker);
        supervisor.open(PrivateServiceConnectionMessage {
            connection_id: Uuid::new_v4(),
            challenge: PrivateServiceChallenge([0; PRIVATE_SERVICE_CHALLENGE_BYTES]),
        });
        assert!(!supervisor.handles.lock().unwrap().is_empty());
        supervisor.cancel();
        supervisor.join_all();
    }
}
