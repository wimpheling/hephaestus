//! Minimal guest bootstrap for approved Hephaestus libkrun images.

use rusqlite::Connection;
use serde::{Serialize, de::DeserializeOwned};
use std::{
    ffi::CString,
    fs::{self, File},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::ffi::OsStrExt,
    os::unix::fs::{OpenOptionsExt, PermissionsExt, chown},
    os::unix::process::{CommandExt, ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;
use vm_libkrun::protocol::{
    GUEST_RUNTIME_AUTHORITY_PATH, GuestCommandMessage, GuestLogStream, GuestMessage,
    GuestStateVolume, HostMessage, MAX_FRAME_SIZE, MAX_PRIVATE_HTTP_BODY_BYTES,
    MAX_PRIVATE_HTTP_HEADERS, PROTOCOL_VERSION, PrivateHttpRequestMessage,
    PrivateHttpResponseMessage, RuntimeAuthorityMessage,
};
use zeroize::Zeroizing;

const CONTROL_CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const PLATFORM_OCI_BUILDER_ENV: &str = "HEPH_PLATFORM_OCI_BUILDER";
const PLATFORM_OCI_BUILDER_PROGRAM: &str = "/usr/libexec/hephaestus/oci-build";
const PLATFORM_OCI_VERIFIER_ENV: &str = "HEPH_PLATFORM_OCI_VERIFIER";
const PLATFORM_OCI_VERIFIER_PROGRAM: &str = "/usr/libexec/hephaestus/oci-verify";
const PLATFORM_OCI_VERIFIER_OPEN_FILES: libc::rlim_t = 65_536;
const AGENT_UID: u32 = 10_001;
const AGENT_GID: u32 = 10_001;
const RUNTIME_AUTHORITY_DIRECTORY: &str = "/run/hephaestus-authority";

fn main() {
    if let Err(error) = run() {
        let _write_result = writeln!(io::stderr(), "heph-init: {error}");
        std::process::exit(125);
    }
}

// Keep guest bootstrap sequencing together so each failure can report while
// the control stream is still open; the resulting function is intentionally
// longer than the pedantic line-count threshold.
#[allow(clippy::too_many_lines)]
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
        state_volume,
        runtime_authority,
        gateway_handler,
    } = read_frame(&mut control)?
    else {
        return Err("host did not send the start command".into());
    };
    if version != PROTOCOL_VERSION {
        return Err(format!("unsupported host protocol version {version}").into());
    }

    // Persist the authority before mounting the immutable runtime control tree
    // at `/run/hephaestus`. A read-only nested virtiofs mount can otherwise
    // make its parent unsuitable for creating the sibling authority directory
    // on some libkrun/FUSE combinations.
    let runtime_authority_ack = runtime_authority
        .as_deref()
        .map(persist_runtime_authority)
        .transpose()?;

    for mount in mounts {
        if let Err(error) = mount_virtiofs(&mount.tag, &mount.guest_path, mount.read_only) {
            let error = io::Error::new(
                error.kind(),
                format!(
                    "mount {} at {}: {error}",
                    mount.tag,
                    mount.guest_path.display()
                ),
            );
            send_guest_error(&mut control, "mount", &error);
            return Err(error.into());
        }
    }
    let mounted_state = match state_volume.as_ref().map(mount_state_volume).transpose() {
        Ok(path) => path,
        Err(error) => {
            send_guest_error(&mut control, "state-volume", &error);
            return Err(error.into());
        }
    };
    if let Some(delay) = command.env.get("HEPH_TEST_READY_DELAY_MS") {
        let milliseconds = delay.parse::<u64>().map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("invalid HEPH_TEST_READY_DELAY_MS: {error}"),
            )
        })?;
        thread::sleep(Duration::from_millis(milliseconds));
    }
    if let Some((session_id, generation)) = runtime_authority_ack {
        write_frame(
            &mut control,
            &GuestMessage::RuntimeAuthorityAcknowledged {
                session_id,
                generation,
            },
        )?;
    }

    if gateway_handler {
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
        gateway_handler_loop(control, &writer, &command)?;
        if let Some(path) = mounted_state {
            unmount(&path)?;
        }
        return Ok(());
    }

    let platform_oci_operation = platform_oci_operation(&command);
    if platform_oci_operation.is_verifier() {
        provision_verifier_open_files()?;
    }
    let mut child = Command::new(&command.program);
    child
        .args(&command.args)
        .env_clear()
        .envs(command.env.iter().filter(|(key, _)| {
            key.as_str() != PLATFORM_OCI_BUILDER_ENV && key.as_str() != PLATFORM_OCI_VERIFIER_ENV
        }))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if !platform_oci_operation.runs_as_root() {
        child.uid(AGENT_UID).gid(AGENT_GID);
    }
    if let Some(working_dir) = command.working_dir {
        child.current_dir(working_dir);
    }
    let mut child = match child.spawn() {
        Ok(child) => child,
        Err(error) => {
            send_guest_error(&mut control, "command-spawn", &error);
            return Err(error.into());
        }
    };

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
    if let Some(path) = mounted_state {
        unmount(&path)?;
    }
    if code == Some(0) && signal.is_none() {
        write_message(
            &writer,
            &GuestMessage::FinalizeResult {
                message: String::from("Hephaestus agent result"),
            },
        )?;
    }
    write_message(&writer, &GuestMessage::Exited { code, signal })?;
    drop(control_thread);
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlatformOciOperation {
    None,
    Builder,
    Verifier,
}

impl PlatformOciOperation {
    const fn runs_as_root(self) -> bool {
        matches!(self, Self::Builder)
    }

    const fn is_verifier(self) -> bool {
        matches!(self, Self::Verifier)
    }
}

fn platform_oci_operation(command: &GuestCommandMessage) -> PlatformOciOperation {
    if command.program == PLATFORM_OCI_BUILDER_PROGRAM
        && command
            .env
            .get(PLATFORM_OCI_BUILDER_ENV)
            .is_some_and(|value| value == "1")
    {
        PlatformOciOperation::Builder
    } else if command.program == PLATFORM_OCI_VERIFIER_PROGRAM
        && command
            .env
            .get(PLATFORM_OCI_VERIFIER_ENV)
            .is_some_and(|value| value == "1")
    {
        PlatformOciOperation::Verifier
    } else {
        PlatformOciOperation::None
    }
}

#[allow(unsafe_code)]
fn provision_verifier_open_files() -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: PLATFORM_OCI_VERIFIER_OPEN_FILES,
        rlim_max: PLATFORM_OCI_VERIFIER_OPEN_FILES,
    };
    // SAFETY: `limit` is initialized, and this bounded root-only bootstrap
    // runs before the exact trusted verifier is spawned.
    let result = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &raw const limit) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[derive(Serialize)]
struct GuestRuntimeAuthority<'a> {
    session_id: uuid::Uuid,
    generation: u64,
    credential_hex: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime_git_credential: Option<&'a str>,
}

fn persist_runtime_authority(
    authority: &RuntimeAuthorityMessage,
) -> Result<(uuid::Uuid, u64), Box<dyn std::error::Error + Send + Sync>> {
    if authority.generation == 0 {
        return Err("runtime authority generation must be positive".into());
    }
    let directory = Path::new(RUNTIME_AUTHORITY_DIRECTORY);
    fs::create_dir_all(directory)
        .map_err(|error| format!("create authority directory: {error}"))?;
    fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
        .map_err(|error| format!("seal authority directory: {error}"))?;
    chown(directory, Some(AGENT_UID), Some(AGENT_GID))
        .map_err(|error| format!("own authority directory: {error}"))?;

    let mut credential_hex = Zeroizing::new(String::with_capacity(authority.credential.len() * 2));
    for byte in &authority.credential {
        use std::fmt::Write as _;
        write!(&mut credential_hex, "{byte:02x}")?;
    }
    let mut runtime_git_credential = authority.runtime_git_credential.as_ref().map(|credential| {
        let mut encoded = String::with_capacity(credential.len() * 2);
        for byte in credential {
            use std::fmt::Write as _;
            // Writing to a String cannot fail.
            let _ = write!(&mut encoded, "{byte:02x}");
        }
        Zeroizing::new(encoded)
    });
    let document = GuestRuntimeAuthority {
        session_id: authority.session_id,
        generation: authority.generation,
        credential_hex: credential_hex.as_str(),
        runtime_git_credential: runtime_git_credential
            .as_deref()
            .map(std::string::String::as_str),
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&document)?);
    match fs::symlink_metadata(GUEST_RUNTIME_AUTHORITY_PATH) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(GUEST_RUNTIME_AUTHORITY_PATH)
                .map_err(|error| format!("remove stale authority credential: {error}"))?;
        }
        Ok(_) => return Err("stale authority credential is not a regular file".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("inspect authority credential: {error}").into()),
    }
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o400)
        .open(GUEST_RUNTIME_AUTHORITY_PATH)
        .map_err(|error| format!("create authority credential: {error}"))?;
    file.write_all(&bytes)
        .map_err(|error| format!("write authority credential: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync authority credential: {error}"))?;
    chown(
        Path::new(GUEST_RUNTIME_AUTHORITY_PATH),
        Some(AGENT_UID),
        Some(AGENT_GID),
    )?;
    if let Some(credential) = &mut runtime_git_credential {
        credential.clear();
    }
    Ok((authority.session_id, authority.generation))
}

fn mount_state_volume(volume: &GuestStateVolume) -> io::Result<PathBuf> {
    let filesystem_uuid = Uuid::parse_str(&volume.filesystem_uuid)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if !volume.guest_path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "state-volume mount path must be absolute",
        ));
    }
    let device = find_ext4_device(filesystem_uuid)?;
    fs::create_dir_all(&volume.guest_path)?;
    mount_ext4(&device, &volume.guest_path)?;
    if volume.guest_path == Path::new("/var/lib/hephaestus") {
        initialize_database(&volume.guest_path)?;
    }
    Ok(volume.guest_path.clone())
}

/// Sends a bounded bootstrap diagnostic while the host control stream is
/// still available. This keeps mount and command failures distinguishable
/// from a worker that disappears before guest initialization completes.
fn send_guest_error(control: &mut File, code: &str, error: &impl std::fmt::Display) {
    let mut message = error.to_string();
    message.truncate(1_024);
    let _write_result = write_frame(
        control,
        &GuestMessage::Error {
            code: code.to_owned(),
            message,
        },
    );
}

fn find_ext4_device(expected: Uuid) -> io::Result<PathBuf> {
    find_ext4_device_in(expected, Path::new("/sys/class/block"), Path::new("/dev"))
}

fn find_ext4_device_in(
    expected: Uuid,
    block_root: &Path,
    device_root: &Path,
) -> io::Result<PathBuf> {
    for entry in fs::read_dir(block_root)? {
        let name = entry?.file_name();
        let device = device_root.join(name);
        let Ok(mut file) = File::open(&device) else {
            continue;
        };
        let mut superblock = [0_u8; 120];
        if file.seek(SeekFrom::Start(1024)).is_err()
            || file.read_exact(&mut superblock).is_err()
            || superblock[56..58] != [0x53, 0xef]
        {
            continue;
        }
        let found = Uuid::from_slice(&superblock[104..120])
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if found == expected {
            return Ok(device);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound,
        format!("ext4 filesystem UUID {expected} was not found"),
    ))
}

fn initialize_database(mount_path: &Path) -> io::Result<()> {
    chown(mount_path, Some(AGENT_UID), Some(AGENT_GID))?;
    let database = mount_path.join("state.db");
    let connection = Connection::open(&database).map_err(io::Error::other)?;
    let mode: String = connection
        .query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))
        .map_err(io::Error::other)?;
    if !mode.eq_ignore_ascii_case("wal") {
        return Err(io::Error::other("SQLite refused WAL journal mode"));
    }
    connection
        .execute_batch("PRAGMA synchronous = FULL;")
        .map_err(io::Error::other)?;
    drop(connection);
    chown(&database, Some(AGENT_UID), Some(AGENT_GID))
}

// Mounting is a privileged operation inside the guest, isolated from the host.
#[allow(unsafe_code)]
fn mount_ext4(source: &Path, target: &Path) -> io::Result<()> {
    let source = CString::new(source.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "disk path contains NUL"))?;
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: all pointers refer to live NUL-terminated strings and the data
    // pointer is null.
    let result = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            c"ext4".as_ptr(),
            libc::MS_NOSUID | libc::MS_NODEV,
            std::ptr::null(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

// Unmounting flushes completed SQLite writes before VM teardown.
#[allow(unsafe_code)]
fn unmount(target: &Path) -> io::Result<()> {
    let target = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "mount path contains NUL"))?;
    // SAFETY: `target` is a live NUL-terminated path and no flags are used.
    let result = unsafe { libc::umount2(target.as_ptr(), 0) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
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

#[derive(Default)]
struct GatewayHandlerState {
    active: bool,
    child_pid: Option<u32>,
    cancelled: bool,
}

/// Serves the private, one-request gateway ABI. The host creates a fresh VM
/// for every public request, so accepting a second request before the first
/// handler has exited is a protocol violation rather than a queue.
fn gateway_handler_loop(
    mut control: File,
    writer: &Arc<Mutex<File>>,
    command: &vm_libkrun::protocol::GuestCommandMessage,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let state = Arc::new(Mutex::new(GatewayHandlerState::default()));
    while let Ok(message) = read_frame::<HostMessage>(&mut control) {
        match message {
            HostMessage::PrivateHttpRequest {
                request_id,
                request,
            } => {
                validate_private_http_request(&request)?;
                let mut current = lock(&state);
                if current.active {
                    return Err("received concurrent private HTTP requests".into());
                }
                current.active = true;
                current.cancelled = false;
                drop(current);
                let writer = Arc::clone(writer);
                let state = Arc::clone(&state);
                let command = command.clone();
                thread::spawn(move || {
                    let result = invoke_gateway_handler(&command, &request, &state);
                    let message = match result {
                        Ok(response) => GuestMessage::PrivateHttpResponse {
                            request_id,
                            response,
                        },
                        Err(error) => GuestMessage::Error {
                            code: String::from("gateway-handler"),
                            message: bounded_error_message(&error),
                        },
                    };
                    let _write_result = write_message(&writer, &message);
                });
            }
            HostMessage::Cancel { .. } => {
                let pid = {
                    let mut current = lock(&state);
                    current.cancelled = true;
                    current.child_pid
                };
                if let Some(pid) = pid {
                    let _signal_result = signal_process(pid, libc::SIGTERM);
                }
            }
            HostMessage::HealthPing { nonce } => {
                write_message(writer, &GuestMessage::Health { nonce })?;
            }
            HostMessage::Start { .. } => return Err("received duplicate start command".into()),
            _ => return Err("received unsupported gateway control message".into()),
        }
    }
    Ok(())
}

fn invoke_gateway_handler(
    command: &vm_libkrun::protocol::GuestCommandMessage,
    request: &PrivateHttpRequestMessage,
    state: &Arc<Mutex<GatewayHandlerState>>,
) -> io::Result<PrivateHttpResponseMessage> {
    let mut child = Command::new(&command.program);
    child
        .args(&command.args)
        .env_clear()
        .envs(&command.env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .uid(AGENT_UID)
        .gid(AGENT_GID);
    if let Some(working_dir) = &command.working_dir {
        child.current_dir(working_dir);
    }
    let mut child = child.spawn()?;
    let pid = child.id();
    {
        let mut current = lock(state);
        current.child_pid = Some(pid);
        if current.cancelled {
            let _signal_result = signal_process(pid, libc::SIGTERM);
        }
    }
    let outcome = (|| {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| io::Error::other("handler stdin is unavailable"))?;
        ciborium::into_writer(request, &mut stdin).map_err(io::Error::other)?;
        drop(stdin);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("handler stdout is unavailable"))?;
        let mut output = Vec::with_capacity(MAX_PRIVATE_HTTP_BODY_BYTES.min(8_192));
        stdout
            .take(u64::try_from(MAX_PRIVATE_HTTP_BODY_BYTES + 1).unwrap())
            .read_to_end(&mut output)?;
        if output.len() > MAX_PRIVATE_HTTP_BODY_BYTES {
            // Stop a malicious handler before waiting: leaving bytes in its
            // stdout pipe could otherwise deadlock the one-request VM.
            let _signal_result = signal_process(pid, libc::SIGKILL);
            let _status = child.wait()?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "gateway handler response exceeds protocol limit",
            ));
        }
        let status = child.wait()?;
        if !status.success() {
            return Err(io::Error::other("gateway handler exited unsuccessfully"));
        }
        let response: PrivateHttpResponseMessage =
            ciborium::from_reader(output.as_slice()).map_err(io::Error::other)?;
        validate_private_http_response(&response)?;
        Ok(response)
    })();
    let mut current = lock(state);
    current.active = false;
    current.child_pid = None;
    drop(current);
    if outcome.is_err() {
        // Every early stdin/stdout/CBOR failure must reap the direct child;
        // otherwise a broken handler pipe leaves a process behind until the
        // microVM is forcibly destroyed.
        let _signal_result = signal_process(pid, libc::SIGKILL);
        let _wait_result = child.wait();
    }
    outcome
}

fn validate_private_http_request(request: &PrivateHttpRequestMessage) -> io::Result<()> {
    if request.method.is_empty()
        || request
            .method
            .bytes()
            .any(|byte| !byte.is_ascii_uppercase())
        || !request.path_and_query.starts_with('/')
        || request.path_and_query.contains('\0')
        || request.headers.len() > MAX_PRIVATE_HTTP_HEADERS
        || request.body.len() > MAX_PRIVATE_HTTP_BODY_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private HTTP request violates gateway contract",
        ));
    }
    Ok(())
}

fn validate_private_http_response(response: &PrivateHttpResponseMessage) -> io::Result<()> {
    if !(100..=599).contains(&response.status)
        || response.headers.len() > MAX_PRIVATE_HTTP_HEADERS
        || response.body.len() > MAX_PRIVATE_HTTP_BODY_BYTES
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "private HTTP response violates gateway contract",
        ));
    }
    Ok(())
}

fn bounded_error_message(error: &io::Error) -> String {
    let mut message = error.to_string();
    message.truncate(1_024);
    message
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

#[cfg(test)]
mod tests {
    use super::{PlatformOciOperation, find_ext4_device_in, platform_oci_operation};
    use std::{
        collections::BTreeMap,
        fs::{self, File},
        io::{Seek, SeekFrom, Write},
    };
    use tempfile::TempDir;
    use uuid::Uuid;
    use vm_libkrun::protocol::GuestCommandMessage;

    #[test]
    fn locates_ext4_device_by_filesystem_uuid() {
        let temp = TempDir::new().unwrap();
        let blocks = temp.path().join("blocks");
        let devices = temp.path().join("devices");
        fs::create_dir(&blocks).unwrap();
        fs::create_dir(&devices).unwrap();
        fs::create_dir(blocks.join("vda")).unwrap();
        fs::create_dir(blocks.join("vdb")).unwrap();
        fs::write(devices.join("vda"), vec![0_u8; 2048]).unwrap();
        let expected = Uuid::new_v4();
        let mut device = File::create(devices.join("vdb")).unwrap();
        device.set_len(2048).unwrap();
        device.seek(SeekFrom::Start(1024 + 56)).unwrap();
        device.write_all(&[0x53, 0xef]).unwrap();
        device.seek(SeekFrom::Start(1024 + 104)).unwrap();
        device.write_all(expected.as_bytes()).unwrap();
        drop(device);

        assert_eq!(
            find_ext4_device_in(expected, &blocks, &devices).unwrap(),
            devices.join("vdb")
        );
    }

    #[test]
    fn only_exact_internal_oci_operations_can_use_guest_root() {
        let command = GuestCommandMessage {
            program: String::from("/usr/libexec/hephaestus/oci-build"),
            args: Vec::new(),
            env: BTreeMap::from([(String::from("HEPH_PLATFORM_OCI_BUILDER"), String::from("1"))]),
            working_dir: None,
        };
        assert_eq!(
            platform_oci_operation(&command),
            PlatformOciOperation::Builder
        );
        assert!(platform_oci_operation(&command).runs_as_root());

        let different_command = GuestCommandMessage {
            program: String::from("/bin/sh"),
            ..command
        };
        assert_eq!(
            platform_oci_operation(&different_command),
            PlatformOciOperation::None
        );

        let verifier = GuestCommandMessage {
            program: String::from("/usr/libexec/hephaestus/oci-verify"),
            args: Vec::new(),
            env: BTreeMap::from([(
                String::from("HEPH_PLATFORM_OCI_VERIFIER"),
                String::from("1"),
            )]),
            working_dir: None,
        };
        assert_eq!(
            platform_oci_operation(&verifier),
            PlatformOciOperation::Verifier
        );
        assert!(!platform_oci_operation(&verifier).runs_as_root());
    }
}
