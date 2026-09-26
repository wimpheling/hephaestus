use super::{
    BrokerInner, HANDSHAKE_BACKOFF, MAX_HANDSHAKE_ATTEMPTS_PER_TURN, PrivateServiceChallenge,
    SERVICE_HANDSHAKE_BYTES, ServiceChallenge, ServiceTransportError,
};
use crate::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC,
    PRIVATE_SERVICE_HANDSHAKE_VERSION,
};
use std::{
    fs, io,
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use subtle::ConstantTimeEq;
use tokio::{
    io::AsyncReadExt,
    net::{UnixListener, UnixStream},
    time::{sleep, timeout},
};
use uuid::Uuid;

pub(super) async fn accept_loop(
    listener: UnixListener,
    inner: Arc<BrokerInner>,
    handshake_timeout: Duration,
) {
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

pub(super) async fn read_handshake(
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

pub(super) fn random_challenge() -> ServiceChallenge {
    let first = Uuid::new_v4();
    let second = Uuid::new_v4();
    let mut challenge = [0_u8; PRIVATE_SERVICE_CHALLENGE_BYTES];
    challenge[..16].copy_from_slice(first.as_bytes());
    challenge[16..].copy_from_slice(second.as_bytes());
    PrivateServiceChallenge(challenge)
}

pub(super) fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    bool::from(left.ct_eq(right))
}

pub(super) fn validate_runtime_dir(runtime_dir: &Path) -> Result<(), ServiceTransportError> {
    let metadata = fs::symlink_metadata(runtime_dir)
        .map_err(|_| ServiceTransportError::InvalidRuntimeDirectory(runtime_dir.to_owned()))?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(ServiceTransportError::InvalidRuntimeDirectory(
            runtime_dir.to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn remove_socket(path: &Path) {
    if let Err(error) = fs::remove_file(path) {
        if error.kind() != io::ErrorKind::NotFound {
            // The runtime directory is private, so cleanup failure is only
            // observable through the provider's later runtime cleanup.
        }
    }
}
