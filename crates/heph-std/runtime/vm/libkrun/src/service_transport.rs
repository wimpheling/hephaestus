//! Authenticated, bounded host-side transport for one private VM service.

use std::{
    collections::HashMap,
    fmt, io,
    path::PathBuf,
    pin::Pin,
    sync::{Arc, Mutex, MutexGuard, atomic::AtomicBool},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use crate::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC, PrivateServiceChallenge,
};
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::UnixStream,
    sync::{Notify, OwnedSemaphorePermit, Semaphore, oneshot},
};
use uuid::Uuid;

const SERVICE_HANDSHAKE_BYTES: usize =
    PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES;
const MAX_HANDSHAKE_ATTEMPTS_PER_TURN: usize = 64;
const HANDSHAKE_BACKOFF: Duration = Duration::from_millis(10);

mod broker;
mod connection;
mod handshake;

#[cfg(test)]
// Test handles intentionally stay live across asynchronous shutdown and I/O
// assertions so the lifecycle tests exercise real ownership boundaries.
#[allow(clippy::significant_drop_tightening)]
#[path = "service_transport/tests.rs"]
mod tests;

/// Errors returned by the host-side private service broker.
#[derive(Debug, Error)]
pub(crate) enum ServiceTransportError {
    #[error("private service broker is closed")]
    Closed,
    #[error("private service connection capacity is exhausted")]
    Saturated,
    #[error("private service reservation expired")]
    ReservationExpired,
    #[error("invalid private service broker configuration: {0}")]
    InvalidConfiguration(&'static str),
    #[error("private service handshake rejected")]
    HandshakeRejected,
    #[error("private service runtime directory is not a private directory: {0}")]
    InvalidRuntimeDirectory(PathBuf),
    #[error("private service transport I/O failed: {0}")]
    Io(#[source] io::Error),
}

impl From<io::Error> for ServiceTransportError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// A single-use challenge delivered over the VM control channel.
pub(crate) type ServiceChallenge = PrivateServiceChallenge;

/// Metadata needed to authorize one guest service connection.
pub(crate) struct ServiceConnectionOffer {
    inner: Arc<BrokerInner>,
    id: Uuid,
    challenge: ServiceChallenge,
    receiver:
        Option<oneshot::Receiver<Result<AuthenticatedServiceConnection, ServiceTransportError>>>,
    completed: bool,
}

impl ServiceConnectionOffer {
    pub(crate) const fn id(&self) -> Uuid {
        self.id
    }

    pub(crate) const fn challenge(&self) -> &ServiceChallenge {
        &self.challenge
    }

    /// Waits for the guest to connect and prove possession of the challenge.
    pub(crate) async fn connect(
        mut self,
    ) -> Result<AuthenticatedServiceConnection, ServiceTransportError> {
        let receiver = self
            .receiver
            .take()
            .expect("a service connection offer can be consumed once");
        let result = receiver.await.map_err(|_| ServiceTransportError::Closed)?;
        self.completed = true;
        result
    }
}

impl fmt::Debug for ServiceConnectionOffer {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ServiceConnectionOffer")
            .field("id", &self.id)
            .field("challenge", &self.challenge)
            .finish_non_exhaustive()
    }
}

impl Drop for ServiceConnectionOffer {
    fn drop(&mut self) {
        if !self.completed {
            self.inner.cancel_pending(self.id);
        }
    }
}

/// An authenticated raw full-duplex Unix stream for one service connection.
pub(crate) struct AuthenticatedServiceConnection {
    inner: Arc<BrokerInner>,
    id: Uuid,
    state: Arc<ConnectionState>,
}

impl fmt::Debug for AuthenticatedServiceConnection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedServiceConnection")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl AsyncRead for AuthenticatedServiceConnection {
    fn poll_read(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        self.state.poll_read(context, buffer)
    }
}

impl AsyncWrite for AuthenticatedServiceConnection {
    fn poll_write(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        self.state.poll_write(context, buffer)
    }

    fn poll_flush(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.state.poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.state.poll_shutdown(context)
    }
}

impl Drop for AuthenticatedServiceConnection {
    fn drop(&mut self) {
        self.inner.remove_active(self.id);
        self.state.close();
    }
}

struct ConnectionState {
    stream: Mutex<Option<UnixStream>>,
    closed: AtomicBool,
    read_waker: Mutex<Option<std::task::Waker>>,
    write_waker: Mutex<Option<std::task::Waker>>,
}

struct BrokerInner {
    closed: AtomicBool,
    notify: Notify,
    state: Mutex<BrokerState>,
    permits: Arc<Semaphore>,
    reservation_timeout: Duration,
}

#[derive(Default)]
struct BrokerState {
    pending: HashMap<Uuid, PendingConnection>,
    active: HashMap<Uuid, ActiveConnection>,
}

struct PendingConnection {
    challenge: ServiceChallenge,
    sender: oneshot::Sender<Result<AuthenticatedServiceConnection, ServiceTransportError>>,
    permit: OwnedSemaphorePermit,
    expires_at: Instant,
}

struct ActiveConnection {
    state: Arc<ConnectionState>,
    _permit: OwnedSemaphorePermit,
}

pub(crate) use broker::ServiceBroker;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn closed_stream_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::BrokenPipe,
        "private service connection closed",
    )
}
