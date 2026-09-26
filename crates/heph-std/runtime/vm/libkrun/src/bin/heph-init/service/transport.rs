use std::{
    fs::File,
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    os::fd::AsRawFd,
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use vm_libkrun::protocol::{
    PRIVATE_SERVICE_CHALLENGE_BYTES, PRIVATE_SERVICE_HANDSHAKE_MAGIC,
    PRIVATE_SERVICE_HANDSHAKE_VERSION, PrivateServiceConnectionMessage,
};

use super::network;

pub(super) fn reap_finished_locked(handles: &mut Vec<JoinHandle<()>>) {
    let mut index = 0;
    while index < handles.len() {
        if handles[index].is_finished() {
            let handle = handles.swap_remove(index);
            let _ = handle.join();
        } else {
            index += 1;
        }
    }
}

pub(super) fn bridge_streams(local: &TcpStream, host: &File) {
    let Ok(local_reader) = local.try_clone() else {
        return;
    };
    let Ok(host_reader) = host.try_clone() else {
        return;
    };
    let Ok(local_writer) = local.try_clone() else {
        return;
    };
    let Ok(host_writer) = host.try_clone() else {
        return;
    };
    let (sender, receiver) = std::sync::mpsc::sync_channel(2);
    let left_sender = sender.clone();
    let left = thread::spawn(move || {
        let _ = left_sender.send(forward(local_reader, host_writer));
    });
    let right = thread::spawn(move || {
        let _ = sender.send(forward(host_reader, local_writer));
    });
    let first = receiver.recv().unwrap_or(ForwardResult::Error);
    if first == ForwardResult::Error {
        network::shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
        network::shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
    }
    let second = receiver.recv().unwrap_or(ForwardResult::Error);
    if second == ForwardResult::Error {
        network::shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
        network::shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
    }
    let _ = left.join();
    let _ = right.join();
    network::shutdown_fd(local.as_raw_fd(), libc::SHUT_RDWR);
    network::shutdown_fd(host.as_raw_fd(), libc::SHUT_RDWR);
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ForwardResult {
    Eof,
    Error,
}

pub(super) fn forward<R: Read, W: Write + AsRawFd>(mut reader: R, mut writer: W) -> ForwardResult {
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => {
                network::shutdown_fd(writer.as_raw_fd(), libc::SHUT_WR);
                return ForwardResult::Eof;
            }
            Ok(read) if writer.write_all(&buffer[..read]).is_err() => return ForwardResult::Error,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return ForwardResult::Error,
        }
    }
}

pub(super) fn remaining(deadline: std::time::Instant) -> io::Result<Duration> {
    deadline
        .checked_duration_since(std::time::Instant::now())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "service connection timed out"))
}

pub(super) fn connect_loopback(
    address: SocketAddr,
    deadline: std::time::Instant,
    cancelled: &AtomicBool,
) -> io::Result<TcpStream> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "service connection cancelled",
            ));
        }
        let timeout = remaining(deadline)?.min(Duration::from_millis(50));
        match TcpStream::connect_timeout(&address, timeout) {
            Ok(stream) => return Ok(stream),
            Err(error) if error.kind() == io::ErrorKind::TimedOut => {}
            Err(error) => return Err(error),
        }
    }
}

pub(super) fn write_handshake(
    host: &mut File,
    connection: &PrivateServiceConnectionMessage,
) -> io::Result<()> {
    let mut frame = [0_u8; 8 + 1 + 16 + PRIVATE_SERVICE_CHALLENGE_BYTES];
    frame[..PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()]
        .copy_from_slice(&PRIVATE_SERVICE_HANDSHAKE_MAGIC);
    frame[PRIVATE_SERVICE_HANDSHAKE_MAGIC.len()] = PRIVATE_SERVICE_HANDSHAKE_VERSION;
    let id_start = PRIVATE_SERVICE_HANDSHAKE_MAGIC.len() + 1;
    frame[id_start..id_start + 16].copy_from_slice(connection.connection_id.as_bytes());
    frame[id_start + 16..].copy_from_slice(connection.challenge.as_bytes());
    host.write_all(&frame)
}

pub(super) fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
