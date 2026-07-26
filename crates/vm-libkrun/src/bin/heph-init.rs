//! Minimal guest bootstrap for approved Hephaestus libkrun images.

use serde::{Serialize, de::DeserializeOwned};
use std::{
    ffi::CString,
    fs::File,
    io::{self, Read, Write},
    os::unix::ffi::OsStrExt,
    os::unix::process::ExitStatusExt,
    path::Path,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};
use vm_libkrun::protocol::{
    GuestLogStream, GuestMessage, HostMessage, MAX_FRAME_SIZE, PROTOCOL_VERSION,
};

const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn main() {
    if let Err(error) = run() {
        let _write_result = writeln!(io::stderr(), "heph-init: {error}");
        std::process::exit(125);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut control = connect_control()?;
    write_frame(
        &mut control,
        &GuestMessage::Hello {
            version: PROTOCOL_VERSION,
        },
    )?;
    let HostMessage::Start {
        version,
        command,
        mounts,
    } = read_frame(&mut control)?
    else {
        return Err("host did not send the start command".into());
    };
    if version != PROTOCOL_VERSION {
        return Err(format!("unsupported host protocol version {version}").into());
    }

    for mount in mounts {
        mount_virtiofs(&mount.tag, &mount.guest_path, mount.read_only)?;
    }
    if let Some(delay) = command.env.get("HEPH_TEST_READY_DELAY_MS") {
        let milliseconds = delay.parse::<u64>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid HEPH_TEST_READY_DELAY_MS: {error}"),
            )
        })?;
        thread::sleep(Duration::from_millis(milliseconds));
    }

    let mut child = Command::new(&command.program);
    child
        .args(&command.args)
        .env_clear()
        .envs(&command.env)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(working_dir) = command.working_dir {
        child.current_dir(working_dir);
    }
    let mut child = child.spawn()?;

    let writer = Arc::new(Mutex::new(control.try_clone()?));
    write_message(&writer, &GuestMessage::Ready)?;
    write_message(
        &writer,
        &GuestMessage::Metric {
            name: String::from("heph_init.ready"),
            value: 1.0,
            labels: std::collections::BTreeMap::from([(
                String::from("protocol"),
                PROTOCOL_VERSION.to_string(),
            )]),
        },
    )?;
    let stdout = child
        .stdout
        .take()
        .ok_or("command stdout pipe is unavailable")?;
    let stderr = child
        .stderr
        .take()
        .ok_or("command stderr pipe is unavailable")?;
    let stdout_thread = pump_logs(stdout, GuestLogStream::Stdout, Arc::clone(&writer));
    let stderr_thread = pump_logs(stderr, GuestLogStream::Stderr, Arc::clone(&writer));
    let control_thread = handle_host_messages(control, Arc::clone(&writer), child.id());

    let status = wait_command(&mut child)?;
    join_log_thread(stdout_thread)?;
    join_log_thread(stderr_thread)?;
    let (code, signal) = exit_parts(status);
    write_message(&writer, &GuestMessage::Exited { code, signal })?;
    drop(control_thread);
    Ok(())
}

fn connect_control() -> io::Result<File> {
    let deadline = Instant::now() + CONTROL_CONNECT_TIMEOUT;
    loop {
        match vsock::connect_host(vm_libkrun::protocol::GUEST_VSOCK_PORT) {
            Ok(stream) => return Ok(stream),
            Err(_) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(50));
            }
            Err(error) => return Err(error),
        }
    }
}

// Mounting is a privileged operation inside the guest, isolated from the host.
#[allow(unsafe_code)]
fn mount_virtiofs(tag: &str, guest_path: &Path, read_only: bool) -> io::Result<()> {
    std::fs::create_dir_all(guest_path)?;
    let source = CString::new(tag)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount tag contains NUL"))?;
    let target = CString::new(guest_path.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    let filesystem = c"virtiofs";
    let mut flags = libc::MS_NOSUID | libc::MS_NODEV;
    if read_only {
        flags |= libc::MS_RDONLY;
    }
    // SAFETY: all pointers refer to live NUL-terminated strings, the optional
    // data pointer is null, and mount flags contain only Linux constants.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            filesystem.as_ptr(),
            flags,
            std::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn pump_logs(
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

fn handle_host_messages(
    mut control: File,
    writer: Arc<Mutex<File>>,
    child_pid: u32,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        while let Ok(message) = read_frame::<HostMessage>(&mut control) {
            match message {
                HostMessage::Cancel { .. } => {
                    let _signal_result = signal_process(child_pid, libc::SIGTERM);
                }
                HostMessage::HealthPing { nonce } => {
                    let _write_result = write_message(&writer, &GuestMessage::Health { nonce });
                }
                _ => {}
            }
        }
    })
}

fn wait_command(child: &mut Child) -> io::Result<ExitStatus> {
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn join_log_thread(thread: thread::JoinHandle<io::Result<()>>) -> io::Result<()> {
    thread
        .join()
        .map_err(|_| io::Error::other("log forwarding thread panicked"))?
}

fn exit_parts(status: ExitStatus) -> (Option<i32>, Option<i32>) {
    (status.code(), status.signal())
}

fn write_message(writer: &Mutex<File>, message: &GuestMessage) -> io::Result<()> {
    write_frame(&mut *lock(writer), message)
}

fn write_frame<T: Serialize>(writer: &mut impl Write, message: &T) -> io::Result<()> {
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

fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// This one syscall is the guest cancellation boundary; all other guest
// bootstrap code remains safe Rust.
#[allow(unsafe_code)]
fn signal_process(pid: u32, signal: i32) -> io::Result<()> {
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

mod vsock {
    #![allow(unsafe_code)]

    use std::{
        fs::File,
        io,
        mem::size_of,
        os::fd::{AsRawFd, FromRawFd, OwnedFd},
    };

    pub fn connect_host(port: u32) -> io::Result<File> {
        // SAFETY: `socket` has no pointer arguments. The returned descriptor is
        // immediately placed in `OwnedFd` to ensure it is closed on errors.
        let raw_fd =
            unsafe { libc::socket(libc::AF_VSOCK, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
        if raw_fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw_fd` is a newly returned, uniquely owned descriptor.
        let socket = unsafe { OwnedFd::from_raw_fd(raw_fd) };
        let address = libc::sockaddr_vm {
            svm_family: libc::sa_family_t::try_from(libc::AF_VSOCK)
                .expect("AF_VSOCK fits sa_family_t"),
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
        if result == 0 {
            Ok(File::from(socket))
        } else {
            Err(io::Error::last_os_error())
        }
    }
}
