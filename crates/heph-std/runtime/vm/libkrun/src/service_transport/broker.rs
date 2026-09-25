use super::{
    ActiveConnection, AuthenticatedServiceConnection, BrokerInner, BrokerState, ConnectionState,
    PendingConnection, ServiceChallenge, ServiceConnectionOffer, ServiceTransportError,
    handshake::{
        accept_loop, constant_time_equal, random_challenge, remove_socket, validate_runtime_dir,
    },
    lock,
};
use crate::protocol::PRIVATE_SERVICE_SOCKET_NAME;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tokio::net::UnixStream;
use tokio::{
    net::UnixListener,
    sync::{Notify, Semaphore, oneshot},
    task::JoinHandle,
};
use uuid::Uuid;

/// Parent-owned Unix broker for authenticated private service connections.
pub(crate) struct ServiceBroker {
    pub(super) inner: Arc<BrokerInner>,
    socket_path: PathBuf,
    accept_task: Mutex<Option<JoinHandle<()>>>,
}

impl ServiceBroker {
    /// Binds a private listener in an existing 0700 VM runtime directory.
    pub(crate) fn bind(
        runtime_dir: &Path,
        max_connections: u32,
        handshake_timeout: Duration,
    ) -> Result<Self, ServiceTransportError> {
        validate_runtime_dir(runtime_dir)?;
        if !(1..=64).contains(&max_connections) || handshake_timeout.is_zero() {
            return Err(ServiceTransportError::InvalidConfiguration(
                "connection limit must be 1..=64 and handshake timeout must be positive",
            ));
        }

        let socket_path = runtime_dir.join(PRIVATE_SERVICE_SOCKET_NAME);
        let listener = UnixListener::bind(&socket_path)?;
        if let Err(error) = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600)) {
            drop(listener);
            let _remove_result = fs::remove_file(&socket_path);
            return Err(error.into());
        }

        let inner = Arc::new(BrokerInner {
            closed: AtomicBool::new(false),
            notify: Notify::new(),
            state: Mutex::new(BrokerState::default()),
            permits: Arc::new(Semaphore::new(max_connections as usize)),
            reservation_timeout: handshake_timeout,
        });
        let task_inner = Arc::clone(&inner);
        let accept_task = tokio::spawn(async move {
            accept_loop(listener, task_inner, handshake_timeout).await;
        });

        Ok(Self {
            inner,
            socket_path,
            accept_task: Mutex::new(Some(accept_task)),
        })
    }

    #[cfg(test)]
    pub(crate) fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Reserves one bounded connection slot and creates its single-use proof.
    pub(crate) fn reserve(&self) -> Result<ServiceConnectionOffer, ServiceTransportError> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err(ServiceTransportError::Closed);
        }
        let permit = Arc::clone(&self.inner.permits)
            .try_acquire_owned()
            .map_err(|_| ServiceTransportError::Saturated)?;
        let (sender, receiver) = oneshot::channel();
        let (id, challenge) = {
            let mut state = lock(&self.inner.state);
            if self.inner.closed.load(Ordering::Acquire) {
                return Err(ServiceTransportError::Closed);
            }
            loop {
                let id = Uuid::new_v4();
                if let std::collections::hash_map::Entry::Vacant(entry) = state.pending.entry(id) {
                    let challenge = random_challenge();
                    entry.insert(PendingConnection {
                        challenge: challenge.clone(),
                        sender,
                        permit,
                        expires_at: Instant::now() + self.inner.reservation_timeout,
                    });
                    break (id, challenge);
                }
            }
        };
        Ok(ServiceConnectionOffer {
            inner: Arc::clone(&self.inner),
            id,
            challenge,
            receiver: Some(receiver),
            completed: false,
        })
    }

    /// Closes the listener, pending reservations, and active streams.
    pub(crate) async fn shutdown(&self) {
        self.inner.close_all();
        self.inner.notify.notify_one();
        let task = lock(&self.accept_task).take();
        if let Some(task) = task {
            let _join_result = task.await;
        }
        remove_socket(&self.socket_path);
    }
}

impl Drop for ServiceBroker {
    fn drop(&mut self) {
        self.inner.close_all();
        self.inner.notify.notify_one();
        let task = lock(&self.accept_task).take();
        if let Some(task) = task {
            task.abort();
        }
        remove_socket(&self.socket_path);
    }
}

impl BrokerInner {
    pub(super) fn cancel_pending(&self, id: Uuid) {
        let _pending = lock(&self.state).pending.remove(&id);
    }

    // The map must be drained after collecting IDs because removal while
    // iterating would invalidate the pending-entry traversal.
    #[allow(clippy::needless_collect)]
    pub(super) fn expire_stale(&self) {
        let expired = {
            let mut state = lock(&self.state);
            let now = Instant::now();
            let ids = state
                .pending
                .iter()
                .filter_map(|(id, pending)| (pending.expires_at <= now).then_some(*id))
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| state.pending.remove(&id))
                .collect::<Vec<_>>()
        };
        for pending in expired {
            let _send_result = pending
                .sender
                .send(Err(ServiceTransportError::ReservationExpired));
        }
    }

    pub(super) fn dispatch(
        self: &Arc<Self>,
        id: Uuid,
        challenge: &ServiceChallenge,
        stream: UnixStream,
    ) {
        if self.closed.load(Ordering::Acquire) {
            return;
        }
        let connection_state = Arc::new(ConnectionState::new(stream));
        let Some(sender) = ({
            let mut state = lock(&self.state);
            if self.closed.load(Ordering::Acquire) {
                return;
            }
            let matches = state.pending.get(&id).is_some_and(|pending| {
                pending.expires_at > Instant::now()
                    && constant_time_equal(&pending.challenge.0, &challenge.0)
            });
            if matches {
                let Some(pending) = state.pending.remove(&id) else {
                    return;
                };
                let PendingConnection { sender, permit, .. } = pending;
                state.active.insert(
                    id,
                    ActiveConnection {
                        state: Arc::clone(&connection_state),
                        _permit: permit,
                    },
                );
                Some(sender)
            } else {
                None
            }
        }) else {
            return;
        };
        let connection = AuthenticatedServiceConnection {
            inner: Arc::clone(self),
            id,
            state: connection_state,
        };
        if sender.send(Ok(connection)).is_err() {
            self.remove_active(id);
        }
    }

    pub(super) fn remove_active(&self, id: Uuid) {
        let _active = lock(&self.state).active.remove(&id);
    }

    pub(super) fn close_all(&self) {
        self.closed.store(true, Ordering::Release);
        let (pending, active) = {
            let mut state = lock(&self.state);
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.active),
            )
        };
        for pending in pending.into_values() {
            let _send_result = pending.sender.send(Err(ServiceTransportError::Closed));
        }
        for active in active.into_values() {
            active.state.close();
        }
    }
}
