use crate::{
    ffi::{Context, FfiError},
    framing::{read_sync, write_sync},
    network::{PasstProcess, WorkerNetworkError},
    protocol::{
        GuestCommandMessage, GuestLogStream, GuestMessage, GuestMount, GuestStateVolume,
        HostMessage, MAX_LOG_CHUNK_SIZE, MAX_METRIC_LABELS, MAX_METRIC_TEXT_SIZE,
        MAX_RESULT_MESSAGE_SIZE, PROTOCOL_VERSION,
    },
    validation::{PreparedForward, PreparedSpec},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    error::Error,
    ffi::OsString,
    fs, io,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::Duration,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfiguration {
    pub passt_binary: PathBuf,
    pub libkrun_library: OsString,
    pub service_uid: u32,
    pub service_gid: u32,
    pub startup_timeout: Duration,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerRequest {
    pub request_id: u64,
    pub command: WorkerCommand,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerCommand {
    Configure {
        config: WorkerConfiguration,
        spec: Box<PreparedSpec>,
        runtime_dir: PathBuf,
    },
    Start,
    Cancel {
        timeout_ms: u64,
    },
    Health {
        nonce: u64,
    },
    Destroy,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerMessage {
    Response {
        request_id: u64,
        result: Result<(), WireError>,
    },
    Event(WorkerEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkerEvent {
    Started {
        ingress: Vec<PreparedForward>,
        vmm_pid: u32,
        passt_pid: Option<u32>,
    },
    Ready,
    Log {
        stream: WireLogStream,
        bytes: Vec<u8>,
    },
    Metric {
        name: String,
        value: f64,
        labels: BTreeMap<String, String>,
    },
    Health {
        nonce: u64,
    },
    FinalizeResult {
        message: String,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
    BackendFailure(WireError),
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum WireLogStream {
    Stdout,
    Stderr,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireError {
    pub kind: WireErrorKind,
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum WireErrorKind {
    InvalidSpec,
    Unsupported,
    Unavailable,
    InvalidState,
    Destroyed,
    Backend,
}

pub fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let socket_path = parse_socket_argument()?;
    let stream = UnixStream::connect(socket_path)?;
    let reader = stream.try_clone()?;
    let writer = Arc::new(Mutex::new(stream));
    run(reader, &writer)
}

fn run(
    mut reader: UnixStream,
    writer: &Arc<Mutex<UnixStream>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let mut runtime: Option<WorkerRuntime> = None;
    loop {
        let request: WorkerRequest = read_sync(&mut reader)?;
        match request.command {
            WorkerCommand::Configure {
                config,
                spec,
                runtime_dir,
            } => {
                let result = if runtime.is_some() {
                    Err(WireError::invalid_state("worker is already configured"))
                } else {
                    WorkerRuntime::configure(config, *spec, runtime_dir)
                        .map(|configured| runtime = Some(configured))
                        .map_err(WireError::from)
                };
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Start => {
                let result = runtime
                    .as_mut()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(WorkerRuntime::start);
                match result {
                    Ok(started) => {
                        send_response(writer, request.request_id, Ok(()))?;
                        send_message(writer, &WorkerMessage::Event(started.event.clone()))?;
                        started.spawn(Arc::clone(writer));
                    }
                    Err(error) => {
                        send_response(writer, request.request_id, Err(error))?;
                    }
                }
            }
            WorkerCommand::Cancel { timeout_ms } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.cancel(timeout_ms));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Health { nonce } => {
                let result = runtime
                    .as_ref()
                    .ok_or_else(|| WireError::invalid_state("worker is not configured"))
                    .and_then(|configured| configured.health(nonce));
                send_response(writer, request.request_id, result)?;
            }
            WorkerCommand::Destroy => {
                send_response(writer, request.request_id, Ok(()))?;
                drop(runtime.take());
                return Ok(());
            }
        }
    }
}

struct WorkerRuntime {
    config: WorkerConfiguration,
    spec: PreparedSpec,
    runtime_dir: PathBuf,
    control_listener: Option<UnixListener>,
    guest: Arc<Mutex<Option<UnixStream>>>,
    passt: Option<PasstProcess>,
    started: bool,
}

struct StartedVmm {
    context: Context,
    event: WorkerEvent,
    listener: UnixListener,
    guest: Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
}

impl WorkerRuntime {
    fn configure(
        config: WorkerConfiguration,
        spec: PreparedSpec,
        runtime_dir: PathBuf,
    ) -> Result<Self, WorkerError> {
        if rustix::process::geteuid().as_raw() != config.service_uid
            || rustix::process::getegid().as_raw() != config.service_gid
        {
            return Err(WorkerError::Identity);
        }
        let metadata = fs::metadata(&runtime_dir)?;
        if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(WorkerError::RuntimePermissions);
        }

        // Loading and freeing one context during configuration proves that
        // libkrun and libkrunfw are available before provision succeeds.
        drop(Context::load(&config.libkrun_library)?);

        let control_path = runtime_dir.join("guest-control.sock");
        let _stale_socket = fs::remove_file(&control_path);
        let control_listener = UnixListener::bind(&control_path)?;
        Ok(Self {
            config,
            spec,
            runtime_dir,
            control_listener: Some(control_listener),
            guest: Arc::new(Mutex::new(None)),
            passt: None,
            started: false,
        })
    }

    fn start(&mut self) -> Result<StartedVmm, WireError> {
        if self.started {
            return Err(WireError::invalid_state("worker is already running"));
        }
        let passt = PasstProcess::start(&self.config, &self.spec.network, &self.runtime_dir)
            .map_err(WireError::from)?;
        let passt_socket = passt.as_ref().map(|_| self.runtime_dir.join("passt.sock"));
        let control_path = self.runtime_dir.join("guest-control.sock");
        let context = Context::load(&self.config.libkrun_library).map_err(WireError::from)?;
        if let Err(error) = context.configure(&self.spec, passt_socket.as_deref(), &control_path) {
            drop(passt);
            return Err(WireError::from(error));
        }

        let listener = self
            .control_listener
            .take()
            .ok_or_else(|| WireError::invalid_state("guest control listener is unavailable"))?;
        let ingress = passt
            .as_ref()
            .map_or_else(Vec::new, |process| process.ingress.clone());
        let passt_pid = passt.as_ref().map(PasstProcess::pid);
        self.passt = passt;
        self.started = true;
        Ok(StartedVmm {
            context,
            event: WorkerEvent::Started {
                ingress,
                vmm_pid: std::process::id(),
                passt_pid,
            },
            listener,
            guest: Arc::clone(&self.guest),
            spec: self.spec.clone(),
        })
    }

    fn cancel(&self, timeout_ms: u64) -> Result<(), WireError> {
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(stream, &HostMessage::Cancel { timeout_ms })
            .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }

    fn health(&self, nonce: u64) -> Result<(), WireError> {
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(stream, &HostMessage::HealthPing { nonce })
            .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }
}

impl StartedVmm {
    fn spawn(self, writer: Arc<Mutex<UnixStream>>) {
        spawn_guest_control(self.listener, self.guest, self.spec, Arc::clone(&writer));
        thread::spawn(move || {
            tracing::info!("entering libkrun VMM");
            if let Err(error) = self.context.start_enter() {
                let wire_error = WireError::from(error);
                let _failure = send_message(
                    &writer,
                    &WorkerMessage::Event(WorkerEvent::BackendFailure(wire_error)),
                );
                let _exit = send_message(
                    &writer,
                    &WorkerMessage::Event(WorkerEvent::Exited {
                        code: None,
                        signal: None,
                    }),
                );
            }
        });
    }
}

fn spawn_guest_control(
    listener: UnixListener,
    guest: Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
    writer: Arc<Mutex<UnixStream>>,
) {
    thread::spawn(move || {
        let result = handle_guest(&listener, &guest, spec, &writer);
        if let Err(error) = result {
            tracing::error!(%error, "guest control channel failed");
            let failure = WireError {
                kind: WireErrorKind::Backend,
                code: "guest-control".to_owned(),
                message: error.to_string(),
            };
            let _failure_result = send_message(
                &writer,
                &WorkerMessage::Event(WorkerEvent::BackendFailure(failure)),
            );
            let _exit_result = send_message(
                &writer,
                &WorkerMessage::Event(WorkerEvent::Exited {
                    code: None,
                    signal: None,
                }),
            );
        }
    });
}

fn handle_guest(
    listener: &UnixListener,
    guest_slot: &Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
    writer: &Arc<Mutex<UnixStream>>,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    let (mut stream, _) = listener.accept()?;
    let hello: GuestMessage = read_sync(&mut stream)?;
    if !matches!(
        hello,
        GuestMessage::Hello {
            version: PROTOCOL_VERSION
        }
    ) {
        return Err(Box::new(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest protocol version mismatch",
        )));
    }

    let guest_writer = stream.try_clone()?;
    *lock(guest_slot) = Some(guest_writer);
    let command = GuestCommandMessage {
        program: spec.command.program,
        args: spec.command.args,
        env: spec.command.env,
        working_dir: spec.command.working_dir,
    };
    let mounts = spec
        .mounts
        .into_iter()
        .map(|mount| GuestMount {
            tag: mount.tag,
            guest_path: mount.guest_path,
            read_only: mount.read_only,
        })
        .collect();
    let state_volume = match (
        spec.labels.get("hephaestus.agent-state.filesystem-uuid"),
        spec.labels.get("hephaestus.agent-state.mount-path"),
    ) {
        (Some(filesystem_uuid), Some(guest_path)) => Some(GuestStateVolume {
            filesystem_uuid: filesystem_uuid.clone(),
            guest_path: PathBuf::from(guest_path),
        }),
        _ => None,
    };
    write_sync(
        &mut stream,
        &HostMessage::Start {
            version: PROTOCOL_VERSION,
            command,
            mounts,
            state_volume,
        },
    )?;

    loop {
        let message: GuestMessage = read_sync(&mut stream)?;
        validate_guest_message(&message)?;
        let event = match message {
            GuestMessage::Hello { .. } => continue,
            GuestMessage::Ready => WorkerEvent::Ready,
            GuestMessage::Log { stream, bytes } => WorkerEvent::Log {
                stream: stream.into(),
                bytes,
            },
            GuestMessage::Metric {
                name,
                value,
                labels,
            } => WorkerEvent::Metric {
                name,
                value,
                labels,
            },
            GuestMessage::Health { nonce } => WorkerEvent::Health { nonce },
            GuestMessage::FinalizeResult { message } => WorkerEvent::FinalizeResult { message },
            GuestMessage::Exited { code, signal } => WorkerEvent::Exited { code, signal },
            GuestMessage::Error { code, message } => WorkerEvent::BackendFailure(WireError {
                kind: WireErrorKind::Backend,
                code,
                message,
            }),
        };
        let exited = matches!(event, WorkerEvent::Exited { .. });
        send_message(writer, &WorkerMessage::Event(event))?;
        if exited {
            break;
        }
    }
    Ok(())
}

fn validate_guest_message(message: &GuestMessage) -> io::Result<()> {
    match message {
        GuestMessage::Hello { .. } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest sent a duplicate readiness handshake",
        )),
        GuestMessage::Log { bytes, .. } if bytes.len() > MAX_LOG_CHUNK_SIZE => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest log chunk exceeds protocol limit",
        )),
        GuestMessage::Metric { name, .. }
            if name.is_empty() || name.len() > MAX_METRIC_TEXT_SIZE =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric name is empty or exceeds protocol limit",
            ))
        }
        GuestMessage::Metric { labels, .. } if labels.len() > MAX_METRIC_LABELS => {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric label count exceeds protocol limit",
            ))
        }
        GuestMessage::Metric { labels, .. }
            if labels.iter().any(|(key, value)| {
                key.is_empty()
                    || key.len() > MAX_METRIC_TEXT_SIZE
                    || value.len() > MAX_METRIC_TEXT_SIZE
            }) =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest metric label is empty or exceeds protocol limit",
            ))
        }
        GuestMessage::FinalizeResult { message }
            if message.len() > MAX_RESULT_MESSAGE_SIZE || message.contains('\0') =>
        {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "guest result message exceeds protocol limit or contains NUL",
            ))
        }
        GuestMessage::Exited {
            code: Some(_),
            signal: Some(_),
        } => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "guest exit cannot contain both code and signal",
        )),
        _ => Ok(()),
    }
}

fn send_response(
    writer: &Arc<Mutex<UnixStream>>,
    request_id: u64,
    result: Result<(), WireError>,
) -> io::Result<()> {
    send_message(writer, &WorkerMessage::Response { request_id, result })
}

fn send_message(writer: &Arc<Mutex<UnixStream>>, message: &WorkerMessage) -> io::Result<()> {
    write_sync(&mut *lock(writer), message)
}

fn parse_socket_argument() -> Result<PathBuf, io::Error> {
    let mut arguments = std::env::args_os();
    let _binary = arguments.next();
    let flag = arguments.next();
    let path = arguments.next();
    if flag.as_deref() != Some(std::ffi::OsStr::new("--socket"))
        || path.is_none()
        || arguments.next().is_some()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "usage: hephaestus-vm-libkrun-worker --socket PATH",
        ));
    }
    Ok(PathBuf::from(path.expect("checked above")))
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl WireError {
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self {
            kind: WireErrorKind::InvalidState,
            code: "invalid-state".to_owned(),
            message: message.into(),
        }
    }

    fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: WireErrorKind::Unavailable,
            code: "unavailable".to_owned(),
            message: message.into(),
        }
    }

    fn io(error: &io::Error) -> Self {
        Self {
            kind: WireErrorKind::Backend,
            code: "worker-io".to_owned(),
            message: error.to_string(),
        }
    }
}

impl From<FfiError> for WireError {
    fn from(error: FfiError) -> Self {
        Self {
            kind: WireErrorKind::Backend,
            code: error.diagnostic_code(),
            message: error.to_string(),
        }
    }
}

impl From<WorkerNetworkError> for WireError {
    fn from(error: WorkerNetworkError) -> Self {
        Self {
            kind: WireErrorKind::Unavailable,
            code: "passt".to_owned(),
            message: error.to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum WorkerError {
    #[error("worker effective identity does not match configuration")]
    Identity,
    #[error("worker runtime directory must be private (mode 0700)")]
    RuntimePermissions,
    #[error("worker I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Ffi(#[from] FfiError),
}

impl From<WorkerError> for WireError {
    fn from(error: WorkerError) -> Self {
        let kind = match error {
            WorkerError::Identity | WorkerError::RuntimePermissions => WireErrorKind::InvalidSpec,
            WorkerError::Io(_) | WorkerError::Ffi(_) => WireErrorKind::Unavailable,
        };
        Self {
            kind,
            code: "worker-configuration".to_owned(),
            message: error.to_string(),
        }
    }
}

impl From<GuestLogStream> for WireLogStream {
    fn from(stream: GuestLogStream) -> Self {
        match stream {
            GuestLogStream::Stdout => Self::Stdout,
            GuestLogStream::Stderr => Self::Stderr,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GuestLogStream, GuestMessage, MAX_LOG_CHUNK_SIZE, MAX_METRIC_LABELS, MAX_METRIC_TEXT_SIZE,
        PROTOCOL_VERSION, validate_guest_message,
    };
    use std::collections::BTreeMap;

    #[test]
    fn exit_with_code_and_signal_is_rejected() {
        assert!(
            validate_guest_message(&GuestMessage::Exited {
                code: Some(1),
                signal: Some(9),
            })
            .is_err()
        );
    }

    #[test]
    fn duplicate_hello_is_rejected_after_handshake() {
        assert!(
            validate_guest_message(&GuestMessage::Hello {
                version: PROTOCOL_VERSION,
            })
            .is_err()
        );
    }

    #[test]
    fn oversized_log_is_rejected() {
        assert!(
            validate_guest_message(&GuestMessage::Log {
                stream: GuestLogStream::Stdout,
                bytes: vec![0; MAX_LOG_CHUNK_SIZE + 1],
            })
            .is_err()
        );
    }

    #[test]
    fn metric_bounds_are_enforced() {
        let oversized_name = GuestMessage::Metric {
            name: "x".repeat(MAX_METRIC_TEXT_SIZE + 1),
            value: 1.0,
            labels: BTreeMap::new(),
        };
        assert!(validate_guest_message(&oversized_name).is_err());

        let labels = (0..=MAX_METRIC_LABELS)
            .map(|index| (format!("key-{index}"), String::from("value")))
            .collect();
        assert!(
            validate_guest_message(&GuestMessage::Metric {
                name: String::from("metric"),
                value: 1.0,
                labels,
            })
            .is_err()
        );

        let invalid_label = GuestMessage::Metric {
            name: String::from("metric"),
            value: 1.0,
            labels: BTreeMap::from([(String::new(), String::from("value"))]),
        };
        assert!(validate_guest_message(&invalid_label).is_err());
    }
}
