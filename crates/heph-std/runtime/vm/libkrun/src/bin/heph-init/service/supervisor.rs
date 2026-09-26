use std::{
    collections::HashMap,
    io,
    net::SocketAddr,
    os::fd::{AsRawFd, OwnedFd},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use vm_libkrun::protocol::{PRIVATE_SERVICE_VSOCK_PORT, PrivateServiceConnectionMessage};

use super::{JOIN_TIMEOUT, SERVICE_IDLE_TIMEOUT, config::ServiceConfig, network, transport};
use crate::vsock;

use super::transport::lock;

#[derive(Debug)]
pub(super) struct State {
    pub(super) closing: bool,
    pub(super) next_id: u64,
    pub(super) active: HashMap<u64, Vec<Arc<OwnedFd>>>,
    pub(super) pending: usize,
}

#[derive(Clone)]
pub struct ServiceSupervisor {
    pub(super) inner: Arc<Inner>,
    pub(super) handles: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

pub(super) struct Inner {
    pub(super) config: ServiceConfig,
    pub(super) state: Mutex<State>,
    pub(super) slots: Condvar,
    pub(super) cancelled: AtomicBool,
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
        transport::reap_finished_locked(&mut handles);
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
            network::shutdown_fd(fd.as_raw_fd(), libc::SHUT_RDWR);
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

    pub(super) fn reap_finished(&self) {
        let mut handles = lock(&self.handles);
        transport::reap_finished_locked(&mut handles);
    }

    pub(super) fn reserve_pending(&self) -> Option<u64> {
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

    pub(super) fn acquire_active(&self, id: u64, deadline: std::time::Instant) -> io::Result<()> {
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
            let wait = match transport::remaining(deadline) {
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

    pub(super) fn register_fds(&self, id: u64, fds: Vec<Arc<OwnedFd>>) -> bool {
        let mut state = lock(&self.inner.state);
        if state.closing {
            return false;
        }
        state.active.get_mut(&id).is_some_and(|active| {
            *active = fds;
            true
        })
    }

    pub(super) fn release(&self, id: u64) {
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
            transport::remaining(deadline)?,
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
        let local = transport::connect_loopback(address, deadline, &self.inner.cancelled)?;
        network::set_socket_timeout(host.as_raw_fd(), libc::SO_RCVTIMEO, SERVICE_IDLE_TIMEOUT)?;
        network::set_socket_timeout(
            host.as_raw_fd(),
            libc::SO_SNDTIMEO,
            transport::remaining(deadline)?,
        )?;
        local.set_read_timeout(Some(SERVICE_IDLE_TIMEOUT))?;
        local.set_write_timeout(Some(SERVICE_IDLE_TIMEOUT))?;
        if self.inner.cancelled.load(Ordering::Acquire) {
            return Ok(());
        }
        transport::write_handshake(&mut host, connection)?;
        network::set_socket_timeout(host.as_raw_fd(), libc::SO_SNDTIMEO, SERVICE_IDLE_TIMEOUT)?;
        let local_guard: OwnedFd = local.try_clone()?.into();
        let local_fd = local.as_raw_fd();
        let host_fd = host.as_raw_fd();
        if !self.register_fds(id, vec![Arc::new(local_guard), host_guard]) {
            network::shutdown_fd(local_fd, libc::SHUT_RDWR);
            network::shutdown_fd(host_fd, libc::SHUT_RDWR);
            return Ok(());
        }
        transport::bridge_streams(&local, &host);
        Ok(())
    }
}
