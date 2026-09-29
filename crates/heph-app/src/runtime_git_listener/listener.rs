use axum::Router;
use std::{
    io,
    os::unix::{
        fs::{FileTypeExt, MetadataExt, PermissionsExt},
        net::UnixListener as StdUnixListener,
    },
    path::{Path, PathBuf},
};
use tokio::net::UnixListener;

const SOCKET_MODE: u32 = 0o600;

/// A bound owner-only runtime Git listener.
pub struct RuntimeGitListener {
    listener: Option<UnixListener>,
    path: PathBuf,
    device: u64,
    inode: u64,
}

impl RuntimeGitListener {
    /// Binds a safe owner-only Unix socket, removing only an owned stale socket.
    pub fn bind(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        validate_socket_path(&path)?;
        remove_owned_stale_socket(&path)?;
        let listener = StdUnixListener::bind(&path)?;
        listener.set_nonblocking(true)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(SOCKET_MODE))?;
        let metadata = std::fs::symlink_metadata(&path)?;
        let listener = UnixListener::from_std(listener)?;
        Ok(Self {
            listener: Some(listener),
            path,
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }

    /// Serves the shared Git router until cancellation, then removes the
    /// socket only if its original inode is still present.
    pub async fn serve(
        mut self,
        router: Router,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> io::Result<()> {
        axum::serve(
            self.listener
                .take()
                .expect("runtime Git listener is served only once"),
            router,
        )
        .with_graceful_shutdown(cancellation.cancelled_owned())
        .await
        .map_err(io::Error::other)
    }
}

impl Drop for RuntimeGitListener {
    fn drop(&mut self) {
        let Ok(metadata) = std::fs::symlink_metadata(&self.path) else {
            return;
        };
        if metadata.file_type().is_socket()
            && metadata.uid() == rustix::process::geteuid().as_raw()
            && metadata.dev() == self.device
            && metadata.ino() == self.inode
        {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn validate_socket_path(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git socket path must be absolute",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "runtime Git socket has no parent",
        )
    })?;
    let metadata = std::fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o022 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime Git socket parent is not a private owner directory",
        ));
    }
    Ok(())
}

fn remove_owned_stale_socket(path: &Path) -> io::Result<()> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_socket() {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "runtime Git socket path is occupied by a non-socket",
        ));
    }
    if metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime Git socket is owned by another user",
        ));
    }
    match std::os::unix::net::UnixStream::connect(path) {
        Ok(stream) => {
            drop(stream);
            Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "runtime Git socket is live",
            ))
        }
        Err(error) if matches!(error.kind(), io::ErrorKind::ConnectionRefused) => {
            std::fs::remove_file(path)
        }
        Err(error) => Err(error),
    }
}
