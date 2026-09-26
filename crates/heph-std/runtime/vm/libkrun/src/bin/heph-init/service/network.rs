use std::{io, os::fd::RawFd, time::Duration};

#[allow(unsafe_code)]
pub(super) fn shutdown_fd(fd: RawFd, how: libc::c_int) {
    // SAFETY: descriptors are borrowed from live socket owners in the active
    // registry; shutdown only changes their connection state.
    let _ = unsafe { libc::shutdown(fd, how) };
}

#[allow(unsafe_code)]
pub(super) fn set_socket_timeout(
    fd: RawFd,
    option: libc::c_int,
    timeout: Duration,
) -> io::Result<()> {
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
pub(super) fn bring_up_loopback_on_socket(socket: RawFd) -> io::Result<()> {
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

pub fn loopback_flags_up(flags: libc::c_short) -> libc::c_short {
    flags | libc::c_short::try_from(libc::IFF_UP).expect("IFF_UP fits c_short")
}
