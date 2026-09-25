/// Bound broker listener ready to accept provider-forwarded vsock streams.
use super::{
    common::{
        Arc, AsyncReadExt, AsyncWriteExt, CancellationToken, FileTypeExt, MAX_FRAME_BYTES, Path,
        PathBuf, PermissionsExt, UnixListener, UnixStream, fs,
    },
    executor::BrokerExecutor,
};

/// Bound broker listener ready to accept provider-forwarded vsock streams.
pub struct BrokerServer {
    socket_path: PathBuf,
    listener: UnixListener,
    executor: Arc<dyn BrokerExecutor>,
}

impl Drop for BrokerServer {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.socket_path)
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            drop(fs::remove_file(&self.socket_path));
        }
    }
}

impl BrokerServer {
    /// Binds a private host socket.
    ///
    /// # Errors
    ///
    /// Rejects non-absolute paths, unsafe existing objects, filesystem
    /// failures, and sockets whose permissions cannot be restricted.
    pub fn bind(
        socket_path: impl Into<PathBuf>,
        executor: Arc<dyn BrokerExecutor>,
    ) -> Result<Self, BrokerServerError> {
        let socket_path = socket_path.into();
        if !socket_path.is_absolute() {
            return Err(BrokerServerError::UnsafeSocket);
        }
        if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
            if !metadata.file_type().is_socket() || metadata.file_type().is_symlink() {
                return Err(BrokerServerError::UnsafeSocket);
            }
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            socket_path,
            listener,
            executor,
        })
    }

    /// Returns the exact provider-facing socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Serves connections until cancellation.
    ///
    /// # Errors
    ///
    /// Returns listener failures. Individual malformed connections are denied
    /// and closed without stopping the listener.
    pub async fn serve(self, cancellation: CancellationToken) -> Result<(), BrokerServerError> {
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let executor = Arc::clone(&self.executor);
                    tokio::spawn(async move {
                        if let Err(error) = serve_connection(stream, executor).await {
                            tracing::warn!(error_class = error.code(), "broker connection closed");
                        }
                    });
                }
            }
        }
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    executor: Arc<dyn BrokerExecutor>,
) -> Result<(), BrokerServerError> {
    loop {
        let length = match stream.read_u32().await {
            Ok(length) => usize::try_from(length).map_err(|_| BrokerServerError::OversizedFrame)?,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(BrokerServerError::OversizedFrame);
        }
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload).await?;
        let request = serde_json::from_slice(&payload).map_err(|_| BrokerServerError::Malformed)?;
        let response = executor.execute(request).await;
        let encoded = serde_json::to_vec(&response).map_err(|_| BrokerServerError::Malformed)?;
        write_frame(&mut stream, &encoded).await?;
    }
}

pub(super) async fn write_frame(
    stream: &mut UnixStream,
    payload: &[u8],
) -> Result<(), BrokerServerError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(BrokerServerError::OversizedFrame);
    }
    let length = u32::try_from(payload.len()).map_err(|_| BrokerServerError::OversizedFrame)?;
    stream.write_u32(length).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

/// Broker listener or framing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BrokerServerError {
    /// Socket path or pre-existing object is unsafe.
    #[error("broker socket path is unsafe")]
    UnsafeSocket,
    /// Filesystem or stream operation failed.
    #[error("broker transport I/O failed")]
    Io(#[from] std::io::Error),
    /// Frame exceeded the protocol bound.
    #[error("broker frame exceeds the protocol bound")]
    OversizedFrame,
    /// Frame was not valid canonical protocol JSON.
    #[error("broker frame is malformed")]
    Malformed,
}

impl BrokerServerError {
    const fn code(&self) -> &'static str {
        match self {
            Self::UnsafeSocket => "unsafe_socket",
            Self::Io(_) => "io",
            Self::OversizedFrame => "oversized_frame",
            Self::Malformed => "malformed",
        }
    }
}
