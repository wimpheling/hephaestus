use super::*;
use crate::protocol::PRIVATE_SERVICE_HANDSHAKE_VERSION;
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
    let broker =
        ServiceBroker::bind(directory.path(), 1, Duration::from_millis(50)).expect("bind broker");
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
