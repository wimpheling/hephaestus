use super::{
    errors::WorkerError,
    guest::spawn_guest_control,
    lock, send_message,
    types::{WireError, WorkerConfiguration, WorkerEvent, WorkerMessage},
};
use crate::{
    ffi::Context,
    framing::write_sync,
    network::PasstProcess,
    protocol::{
        GUEST_CONTROL_SOCKET_NAME, HostMessage, PRIVATE_SERVICE_SOCKET_NAME,
        PrivateServiceConnectionMessage,
    },
    validation::PreparedSpec,
};
use std::{
    fs,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{Arc, Mutex},
    thread,
};

pub(super) struct WorkerRuntime {
    config: WorkerConfiguration,
    pub(super) spec: PreparedSpec,
    runtime_dir: PathBuf,
    control_listener: Option<UnixListener>,
    pub(super) guest: Arc<Mutex<Option<UnixStream>>>,
    passt: Option<PasstProcess>,
    started: bool,
}

pub(super) struct StartedVmm {
    pub(super) context: Context,
    pub(super) event: WorkerEvent,
    pub(super) listener: UnixListener,
    pub(super) guest: Arc<Mutex<Option<UnixStream>>>,
    spec: PreparedSpec,
}

impl WorkerRuntime {
    pub(super) fn configure(
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

        let control_path = runtime_dir.join(GUEST_CONTROL_SOCKET_NAME);
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

    pub(super) fn start(&mut self) -> Result<StartedVmm, WireError> {
        if self.started {
            return Err(WireError::invalid_state("worker is already running"));
        }
        let passt = PasstProcess::start(&self.config, &self.spec.network, &self.runtime_dir)
            .map_err(WireError::from)?;
        let passt_socket = passt.as_ref().map(|_| self.runtime_dir.join("passt.sock"));
        let control_path = self.runtime_dir.join(GUEST_CONTROL_SOCKET_NAME);
        let context = Context::load(&self.config.libkrun_library).map_err(WireError::from)?;
        if let Err(error) = context.configure(
            &self.spec,
            passt_socket.as_deref(),
            &control_path,
            self.config.broker_socket_path.as_deref(),
            self.spec
                .private_http_service
                .as_ref()
                .map(|_| self.runtime_dir.join(PRIVATE_SERVICE_SOCKET_NAME))
                .as_deref(),
            self.spec
                .runtime_git_bridge
                .as_ref()
                .and(self.config.runtime_git_socket_path.as_deref()),
        ) {
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

    pub(super) fn cancel(&self, timeout_ms: u64) -> Result<(), WireError> {
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(stream, &HostMessage::Cancel { timeout_ms })
            .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }

    pub(super) fn health(&self, nonce: u64) -> Result<(), WireError> {
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(stream, &HostMessage::HealthPing { nonce })
            .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }

    pub(super) fn private_http(
        &self,
        request_id: u64,
        request: crate::protocol::PrivateHttpRequestMessage,
    ) -> Result<(), WireError> {
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(
            stream,
            &HostMessage::PrivateHttpRequest {
                request_id,
                request,
            },
        )
        .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }

    pub(super) fn open_private_service_connection(
        &self,
        connection: PrivateServiceConnectionMessage,
    ) -> Result<(), WireError> {
        if self.spec.private_http_service.is_none() {
            return Err(WireError::unsupported(
                "private HTTP service is not declared for this VM",
            ));
        }
        let mut guest = lock(&self.guest);
        let stream = guest
            .as_mut()
            .ok_or_else(|| WireError::unavailable("guest control channel is not ready"))?;
        let result = write_sync(
            stream,
            &HostMessage::OpenPrivateServiceConnection { connection },
        )
        .map_err(|error| WireError::io(&error));
        drop(guest);
        result
    }
}

impl StartedVmm {
    pub(super) fn spawn(self, writer: Arc<Mutex<UnixStream>>) {
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
