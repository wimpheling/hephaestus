//! Authenticated, bounded host-side transport for one private VM service.

use std::{
    collections::HashMap,
    fmt, fs, io,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    pin::Pin,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    time::{Duration, Instant},
};

use crate::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC,
    PRIVATE_SERVICE_HANDSHAKE_VERSION, PRIVATE_SERVICE_SOCKET_NAME, PrivateServiceChallenge,
};
use subtle::ConstantTimeEq;
use thiserror::Error;
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, ReadBuf},
    net::{UnixListener, UnixStream},
    sync::{Notify, OwnedSemaphorePermit, Semaphore, oneshot},
    task::JoinHandle,
    time::{sleep, timeout},
};
use uuid::Uuid;

const SERVICE_HANDSHAKE_BYTES: usize =
    PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES;
const MAX_HANDSHAKE_ATTEMPTS_PER_TURN: usize = 64;
const HANDSHAKE_BACKOFF: Duration = Duration::from_millis(10);

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

impl ConnectionState {
    const fn new(stream: UnixStream) -> Self {
        Self {
            stream: Mutex::new(Some(stream)),
            closed: AtomicBool::new(false),
            read_waker: Mutex::new(None),
            write_waker: Mutex::new(None),
        }
    }

    fn poll_read(
        &self,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        {
            let mut waker = lock(&self.read_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Ok(()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = Pin::new(&mut *stream).poll_read(context, buffer);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.read_waker).take();
        }
        result
    }

    fn poll_write(&self, context: &mut Context<'_>, buffer: &[u8]) -> Poll<io::Result<usize>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Err(closed_stream_error()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Err(closed_stream_error()));
        };
        let result = Pin::new(&mut *stream).poll_write(context, buffer);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    fn poll_flush(&self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Err(closed_stream_error()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Err(closed_stream_error()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Err(closed_stream_error()));
        };
        let result = Pin::new(&mut *stream).poll_flush(context);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    fn poll_shutdown(&self, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        {
            let mut waker = lock(&self.write_waker);
            if self.closed.load(Ordering::Acquire) {
                return Poll::Ready(Ok(()));
            }
            *waker = Some(context.waker().clone());
        }
        if self.closed.load(Ordering::Acquire) {
            return Poll::Ready(Ok(()));
        }
        let mut stream_guard = lock(&self.stream);
        let Some(stream) = stream_guard.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = Pin::new(&mut *stream).poll_shutdown(context);
        drop(stream_guard);
        if result.is_ready() {
            lock(&self.write_waker).take();
        }
        result
    }

    fn close(&self) {
        self.closed.store(true, Ordering::Release);
        drop(lock(&self.stream).take());
        let read_waker = lock(&self.read_waker).take();
        let write_waker = lock(&self.write_waker).take();
        if let Some(waker) = read_waker {
            waker.wake();
        }
        if let Some(waker) = write_waker {
            waker.wake();
        }
    }
}

/// Parent-owned Unix broker for authenticated private service connections.
pub(crate) struct ServiceBroker {
    inner: Arc<BrokerInner>,
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

impl BrokerInner {
    fn cancel_pending(&self, id: Uuid) {
        let _pending = lock(&self.state).pending.remove(&id);
    }

    // The map must be drained after collecting IDs because removal while
    // iterating would invalidate the pending-entry traversal.
    #[allow(clippy::needless_collect)]
    fn expire_stale(&self) {
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

    fn dispatch(self: &Arc<Self>, id: Uuid, challenge: &ServiceChallenge, stream: UnixStream) {
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

    fn remove_active(&self, id: Uuid) {
        let _active = lock(&self.state).active.remove(&id);
    }

    fn close_all(&self) {
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

async fn accept_loop(listener: UnixListener, inner: Arc<BrokerInner>, handshake_timeout: Duration) {
    let mut attempts = 0;
    let mut expiration = tokio::time::interval(handshake_timeout.min(Duration::from_millis(100)));
    loop {
        if inner.closed.load(Ordering::Acquire) {
            break;
        }
        if attempts >= MAX_HANDSHAKE_ATTEMPTS_PER_TURN {
            attempts = 0;
            sleep(HANDSHAKE_BACKOFF).await;
        }
        attempts += 1;
        tokio::select! {
            () = inner.notify.notified() => break,
            _ = expiration.tick() => inner.expire_stale(),
            result = listener.accept() => {
                let Ok((stream, _address)) = result else {
                    inner.close_all();
                    break;
                };
                if inner.closed.load(Ordering::Acquire) {
                    break;
                }
                let shutdown = inner.notify.notified();
                let result = tokio::select! {
                    () = shutdown => break,
                    result = timeout(handshake_timeout, read_handshake(stream)) => result,
                };
                if let Ok(Ok((id, challenge, stream))) = result {
                    inner.dispatch(id, &challenge, stream);
                }
            }
        }
    }
}

async fn read_handshake(
    mut stream: UnixStream,
) -> Result<(Uuid, ServiceChallenge, UnixStream), ServiceTransportError> {
    let mut frame = [0_u8; SERVICE_HANDSHAKE_BYTES];
    stream.read_exact(&mut frame).await?;
    if frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] != PRIVATE_SERVICE_HANDSHAKE_MAGIC
        || frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] != PRIVATE_SERVICE_HANDSHAKE_VERSION
    {
        return Err(ServiceTransportError::HandshakeRejected);
    }
    let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
    let id_end = id_start + 16;
    let id = Uuid::from_bytes(
        frame[id_start..id_end]
            .try_into()
            .expect("fixed UUID width"),
    );
    let mut challenge = [0_u8; PRIVATE_SERVICE_CHALLENGE_BYTES];
    challenge.copy_from_slice(&frame[id_end..]);
    Ok((id, PrivateServiceChallenge(challenge), stream))
}

fn random_challenge() -> ServiceChallenge {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut challenge = [0_u8; PRIVATE_SERVICE_CHALLENGE_BYTES];
    challenge[..16].copy_from_slice(first.as_bytes());
    challenge[16..].copy_from_slice(second.as_bytes());
    PrivateServiceChallenge(challenge)
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.ct_eq(right))
}

fn validate_runtime_dir(runtime_dir: &Path) -> Result<(), ServiceTransportError> {
    let metadata = fs::symlink_metadata(runtime_dir)
        .map_err(|_| ServiceTransportError::InvalidRuntimeDirectory(runtime_dir.to_owned()))?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(ServiceTransportError::InvalidRuntimeDirectory(
            runtime_dir.to_owned(),
        ));
    }
    Ok(())
}

fn remove_socket(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != io::ErrorKind::NotFound {
            // The runtime directory is private, so cleanup failure is only
            // observable through the provider's later runtime cleanup.
        }
    }
}

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

#[cfg(test)]
// Test handles intentionally stay live across asynchronous shutdown and I/O
// assertions so the lifecycle tests exercise real ownership boundaries.
#[allow(clippy::significant_drop_tightening)]
mod tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, sync::atomic::Ordering};
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn runtime_dir() -> TempDir {
        let directory = TempDir::new().expect("runtime directory");
        std::fs::set_permissions(directory.path(), std::fs::Permissions::from_mode(0o700))
            .expect("private runtime directory");
        directory
    }

    fn frame(id: Uuid, challenge: &PrivateServiceChallenge) -> [u8; SERVICE_HANDSHAKE_BYTES] {
        let mut frame = [0_u8; SERVICE_HANDSHAKE_BYTES];
        frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()]
            .copy_from_slice(&PRIVATE_SERVICE_HANDSHAKE_MAGIC);
        frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] = PRIVATE_SERVICE_HANDSHAKE_VERSION;
        let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
        frame[id_start..id_start + 16].copy_from_slice(id.as_bytes());
        frame[id_start + 16..].copy_from_slice(challenge.as_bytes());
        frame
    }

    async fn connect_offer(broker: &ServiceBroker, offer: &ServiceConnectionOffer) -> UnixStream {
        let stream = UnixStream::connect(broker.socket_path())
            .await
            .expect("connect to private service listener");
        let mut stream = stream;
        stream
            .write_all(&frame(offer.id(), offer.challenge()))
            .await
            .expect("send service handshake");
        stream
    }

    #[tokio::test]
    async fn authenticates_and_preserves_raw_bidirectional_io() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 2, Duration::from_secs(1)).expect("bind broker");
        let mode = std::fs::metadata(broker.socket_path())
            .expect("socket metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);

        let offer = broker.reserve().expect("reserve connection");
        let mut client = connect_offer(&broker, &offer).await;
        let mut server = offer.connect().await.expect("authenticated connection");
        client.write_all(b"request").await.expect("client write");
        let mut request = [0_u8; 7];
        server.read_exact(&mut request).await.expect("server read");
        assert_eq!(&request, b"request");
        server.write_all(b"response").await.expect("server write");
        let mut response = [0_u8; 8];
        client.read_exact(&mut response).await.expect("client read");
        assert_eq!(&response, b"response");

        broker.shutdown().await;
    }

    #[tokio::test]
    async fn rejects_wrong_nonce_unknown_id_and_replay() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 2, Duration::from_secs(1)).expect("bind broker");
        let offer = broker.reserve().expect("reserve connection");
        let challenge = offer.challenge().clone();
        let mut wrong = UnixStream::connect(broker.socket_path())
            .await
            .expect("wrong connection");
        let mut wrong_challenge = offer.challenge().clone();
        wrong_challenge.0[0] ^= 1;
        wrong
            .write_all(&frame(offer.id(), &wrong_challenge))
            .await
            .expect("wrong handshake");
        drop(wrong);

        let unknown_id = Uuid::new_v4();
        let mut unknown = UnixStream::connect(broker.socket_path())
            .await
            .expect("unknown connection");
        unknown
            .write_all(&frame(unknown_id, offer.challenge()))
            .await
            .expect("unknown handshake");
        drop(unknown);

        let mut client = connect_offer(&broker, &offer).await;
        let mut server = offer.connect().await.expect("correct handshake");
        server.write_all(b"ok").await.expect("server write");
        let mut bytes = [0_u8; 2];
        client.read_exact(&mut bytes).await.expect("client read");
        assert_eq!(&bytes, b"ok");

        let mut replay = UnixStream::connect(broker.socket_path())
            .await
            .expect("replay connection");
        replay
            .write_all(&frame(server.id, &challenge))
            .await
            .expect("replay handshake");
        let mut replay_byte = [0_u8; 1];
        assert_eq!(
            replay.read(&mut replay_byte).await.expect("replay close"),
            0
        );
        broker.shutdown().await;
    }

    #[tokio::test]
    async fn expired_offer_cannot_authenticate_after_accept_is_delayed() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 2, Duration::from_secs(1)).expect("bind broker");
        let offer = broker.reserve().expect("reservation");
        let mut blocker = UnixStream::connect(broker.socket_path())
            .await
            .expect("blocking connection");
        blocker.write_all(&[0]).await.expect("partial handshake");
        tokio::task::yield_now().await;
        {
            let mut state = lock(&broker.inner.state);
            state
                .pending
                .get_mut(&offer.id())
                .expect("pending offer")
                .expires_at = Instant::now()
                .checked_sub(Duration::from_millis(1))
                .expect("instant subtraction");
        }

        let mut client = connect_offer(&broker, &offer).await;
        drop(blocker);
        let result = tokio::time::timeout(Duration::from_millis(250), offer.connect())
            .await
            .expect("expired offer result");
        assert!(matches!(
            result,
            Err(ServiceTransportError::ReservationExpired)
        ));
        let mut byte = [0_u8; 1];
        assert!(matches!(client.read(&mut byte).await, Ok(0) | Err(_)));
        broker.shutdown().await;
    }

    #[tokio::test]
    async fn reservation_cancellation_and_saturation_release_capacity() {
        let directory = runtime_dir();
        let broker = ServiceBroker::bind(directory.path(), 1, Duration::from_millis(50))
            .expect("bind broker");
        let first = broker.reserve().expect("first reservation");
        assert!(matches!(
            broker.reserve(),
            Err(ServiceTransportError::Saturated)
        ));
        drop(first);
        let second = broker.reserve().expect("capacity after cancellation");
        let result = tokio::time::timeout(Duration::from_millis(100), second.connect())
            .await
            .expect("reservation should expire");
        assert!(matches!(
            result,
            Err(ServiceTransportError::ReservationExpired)
        ));
        let _third = broker.reserve().expect("capacity after expiry");
        broker.shutdown().await;
    }

    #[tokio::test]
    async fn active_connections_hold_capacity_until_dropped() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 1, Duration::from_secs(1)).expect("bind broker");
        let offer = broker.reserve().expect("reservation");
        let _client = connect_offer(&broker, &offer).await;
        let server = offer.connect().await.expect("active connection");
        assert!(matches!(
            broker.reserve(),
            Err(ServiceTransportError::Saturated)
        ));
        drop(server);
        let _next = broker.reserve().expect("capacity after active drop");
        broker.shutdown().await;
    }

    #[tokio::test]
    async fn shutdown_closes_pending_and_active_connections() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 2, Duration::from_secs(1)).expect("bind broker");
        let pending = broker.reserve().expect("pending reservation");
        let mut client = connect_offer(&broker, &pending).await;
        let server = pending.connect().await.expect("active connection");
        let pending_after_shutdown = broker.reserve().expect("second reservation");
        let pending_result = tokio::spawn(pending_after_shutdown.connect());
        broker.shutdown().await;
        assert!(matches!(
            pending_result.await.expect("pending join"),
            Err(ServiceTransportError::Closed)
        ));
        let mut bytes = [0_u8; 1];
        assert_eq!(client.read(&mut bytes).await.expect("client close"), 0);
        drop(server);
        assert!(broker.inner.closed.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn shutdown_wakes_a_pending_active_read() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 1, Duration::from_secs(1)).expect("bind broker");
        let offer = broker.reserve().expect("reservation");
        let _client = connect_offer(&broker, &offer).await;
        let mut server = offer.connect().await.expect("active connection");
        let read_task = tokio::spawn(async move {
            let mut byte = [0_u8; 1];
            server.read(&mut byte).await
        });
        tokio::task::yield_now().await;
        tokio::time::timeout(Duration::from_millis(100), broker.shutdown())
            .await
            .expect("shutdown should not wait for a blocked read");
        assert_eq!(read_task.await.expect("read task").expect("read result"), 0);
    }

    #[tokio::test]
    async fn shutdown_cancels_an_incomplete_handshake() {
        let directory = runtime_dir();
        let broker =
            ServiceBroker::bind(directory.path(), 1, Duration::from_secs(30)).expect("bind broker");
        let mut client = UnixStream::connect(broker.socket_path())
            .await
            .expect("incomplete handshake connection");
        tokio::time::timeout(Duration::from_millis(100), broker.shutdown())
            .await
            .expect("shutdown should cancel handshake promptly");
        let mut byte = [0_u8; 1];
        match client.read(&mut byte).await {
            Ok(0) => {}
            Err(error) if error.kind() == io::ErrorKind::ConnectionReset => {}
            result => panic!("unexpected incomplete-handshake result: {result:?}"),
        }
    }
}
