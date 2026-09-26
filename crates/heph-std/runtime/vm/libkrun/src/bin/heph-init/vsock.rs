use std::{
    fs::File,
    io,
    mem::size_of,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub fn connect_host(port: u32) -> io::Result<File> {
    connect_host_with_timeout(port, Duration::from_secs(30))
}

pub fn connect_host_with_timeout(port: u32, timeout: Duration) -> io::Result<File> {
    connect_host_with_cancel(port, timeout, &AtomicBool::new(false))
}

// VSOCK setup requires direct libc calls because no safe VSOCK connector is available.
#[allow(unsafe_code)]
pub fn connect_host_with_cancel(
    port: u32,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> io::Result<File> {
    // SAFETY: `socket` has no pointer arguments. The returned descriptor is
    // immediately placed in `OwnedFd` to ensure it is closed on errors.
    let raw_fd = unsafe {
        libc::socket(
            libc::AF_VSOCK,
            libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            0,
        )
    };
    if raw_fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw_fd` is a newly returned, uniquely owned descriptor.
    let socket = unsafe { OwnedFd::from_raw_fd(raw_fd) };
    let address = libc::sockaddr_vm {
        svm_family: libc::sa_family_t::try_from(libc::AF_VSOCK).expect("AF_VSOCK fits sa_family_t"),
        svm_reserved1: 0,
        svm_port: port,
        svm_cid: libc::VMADDR_CID_HOST,
        svm_zero: [0; 4],
    };
    let length = libc::socklen_t::try_from(size_of::<libc::sockaddr_vm>())
        .expect("sockaddr_vm size fits socklen_t");
    // SAFETY: `address` is initialized as an AF_VSOCK sockaddr and the
    // pointer remains valid for the duration specified by `length`.
    let result = unsafe {
        libc::connect(
            socket.as_raw_fd(),
            (&raw const address).cast::<libc::sockaddr>(),
            length,
        )
    };
    if result != 0 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(error);
        }
        let deadline = std::time::Instant::now() + timeout;
        let mut poll = libc::pollfd {
            fd: socket.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        loop {
            if cancelled.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "service connection cancelled",
                ));
            }
            let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) else {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "vsock connect timed out",
                ));
            };
            let milliseconds = i32::try_from(remaining.as_millis().min(50).min(i32::MAX as u128))
                .expect("poll timeout fits i32");
            let ready = unsafe { libc::poll(&raw mut poll, 1, milliseconds) };
            if ready > 0 {
                break;
            }
            if ready < 0 {
                let error = io::Error::last_os_error();
                if error.kind() != io::ErrorKind::Interrupted {
                    return Err(error);
                }
            }
        }
        let mut socket_error = 0_i32;
        let mut length = libc::socklen_t::try_from(size_of::<i32>())
            .expect("socket option length fits socklen_t");
        if unsafe {
            libc::getsockopt(
                socket.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&raw mut socket_error).cast(),
                &raw mut length,
            )
        } < 0
        {
            return Err(io::Error::last_os_error());
        }
        if socket_error != 0 {
            return Err(io::Error::from_raw_os_error(socket_error));
        }
    }
    let flags = unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(socket.as_raw_fd(), libc::F_SETFL, flags & !libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(File::from(socket))
}
