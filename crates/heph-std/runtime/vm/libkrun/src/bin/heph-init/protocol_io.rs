use serde::{Serialize, de::DeserializeOwned};
use std::{
    fs::File,
    io::{self, Read, Write},
    os::unix::process::ExitStatusExt,
    process::{Child, ExitStatus},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::Duration,
};
use vm_libkrun::protocol::{GuestLogStream, GuestMessage, HostMessage, MAX_FRAME_SIZE};

use crate::service;

pub fn pump_logs(
    mut reader: impl Read + Send + 'static,
    stream: GuestLogStream,
    writer: Arc<Mutex<File>>,
) -> thread::JoinHandle<io::Result<()>> {
    thread::spawn(move || {
        let mut buffer = vec![0_u8; vm_libkrun::protocol::MAX_LOG_CHUNK_SIZE];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            write_message(
                &writer,
                &GuestMessage::Log {
                    stream,
                    bytes: buffer[..read].to_vec(),
                },
            )?;
        }
    })
}

pub fn handle_host_messages(
    mut control: File,
    writer: Arc<Mutex<File>>,
    child_pid: u32,
    service_supervisor: Option<service::ServiceSupervisor>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(message) = read_frame::<HostMessage>(&mut control) {
            match message {
                HostMessage::Cancel { .. } => {
                    if let Some(supervisor) = service_supervisor.as_ref() {
                        supervisor.cancel();
                    }
                    let _signal_result = signal_process(child_pid, libc::SIGTERM);
                }
                HostMessage::HealthPing { nonce } => {
                    let _write_result = write_message(&writer, &GuestMessage::Health { nonce });
                }
                HostMessage::OpenPrivateServiceConnection { connection } => {
                    if let Some(supervisor) = service_supervisor.as_ref() {
                        supervisor.open(connection);
                    }
                }
                _ => {}
            }
        }
        if let Some(supervisor) = service_supervisor {
            supervisor.cancel();
            let _signal_result = signal_process(child_pid, libc::SIGTERM);
        }
    })
}

pub fn wait_command(child: &mut Child) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn join_log_thread(thread: thread::JoinHandle<io::Result<()>>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("log forwarding thread panicked"))?
}

pub fn exit_parts(status: ExitStatus) -> (Option<i32>, Option<i32>) {
    (status.code(), status.signal())
}

pub fn write_message(writer: &Mutex<File>, message: &GuestMessage) -> io::Result<()> {
    write_frame(&mut *lock(writer), message)
}

pub fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
    let mut payload = Vec::new();
    ciborium::into_writer(message, &mut payload).map_err(io::Error::other)?;
    if payload.len() > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds protocol limit",
        ));
    }
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "frame is too large"))?;
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()
}

pub fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut length = [0_u8; 4];
    reader.read_exact(&mut length)?;
    let length = usize::try_from(u32::from_be_bytes(length))
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid frame length"))?;
    if length > MAX_FRAME_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame exceeds protocol limit",
        ));
    }
    let mut payload = vec![0_u8; length];
    reader.read_exact(&mut payload)?;
    ciborium::from_reader(payload.as_slice()).map_err(io::Error::other)
}

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// This one syscall is the guest cancellation boundary; all other guest
// bootstrap code remains safe Rust.
#[allow(unsafe_code)]
pub fn signal_process(pid: u32, signal: i32) -> io::Result<()> {
    let pid = i32::try_from(pid)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "child PID exceeds i32"))?;
    // SAFETY: `pid` identifies the directly spawned child and `signal` is a
    // valid Linux signal constant. `kill` does not dereference memory.
    let result = unsafe { libc::kill(pid, signal) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}
