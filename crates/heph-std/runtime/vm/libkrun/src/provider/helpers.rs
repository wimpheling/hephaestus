use super::common::{
    Error, ExitStatus, ExitStatusExt, PROVIDER_NAME, Path, PathBuf, PermissionsExt, PortForward,
    PortProtocol, PreparedForward, VmError, VmEvent, VmExit, VmId, WireError, WireErrorKind,
    broadcast, fmt, fs, io,
};

#[derive(Debug, Clone)]
pub(super) struct ProcessStatus {
    pub(super) code: Option<i32>,
    pub(super) signal: Option<i32>,
}

impl ProcessStatus {
    pub(super) const fn into_vm_exit(self) -> VmExit {
        VmExit {
            code: self.code,
            signal: self.signal,
        }
    }
}

impl From<ExitStatus> for ProcessStatus {
    fn from(status: ExitStatus) -> Self {
        Self {
            code: status.code(),
            signal: status.signal(),
        }
    }
}

pub(super) fn create_runtime_dir(root: &Path, id: &str) -> Result<PathBuf, VmError> {
    let path = root.join(id);
    fs::create_dir(&path).map_err(|error| match error.kind() {
        io::ErrorKind::AlreadyExists => VmError::AlreadyExists(VmId(id.to_owned())),
        _ => provider_error("runtime-create", error),
    })?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .map_err(|error| provider_error("runtime-permissions", error))?;
    Ok(path)
}

pub(super) fn cleanup_runtime(path: &Path) -> Result<(), VmError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(provider_error("runtime-cleanup", error)),
    }
}

pub(super) const fn to_port_forward(forward: PreparedForward) -> PortForward {
    PortForward {
        protocol: PortProtocol::Tcp,
        bind_addr: forward.bind_addr,
        host_port: forward.host_port,
        guest_port: forward.guest_port,
    }
}

pub(super) fn send_event(sender: &broadcast::Sender<VmEvent>, event: VmEvent) {
    drop(sender.send(event));
}

pub(super) fn wire_to_vm_error(error: WireError) -> VmError {
    match error.kind {
        WireErrorKind::InvalidSpec => VmError::InvalidSpec {
            field: error.code,
            reason: error.message,
        },
        WireErrorKind::Unsupported => VmError::Unsupported {
            feature: error.message,
            provider: PROVIDER_NAME.to_owned(),
        },
        WireErrorKind::Unavailable => VmError::Unavailable {
            resource: error.code,
            reason: error.message,
        },
        WireErrorKind::InvalidState => VmError::InvalidState("worker rejected lifecycle operation"),
        WireErrorKind::Destroyed => VmError::Destroyed,
        WireErrorKind::Backend => VmError::Provider {
            provider: PROVIDER_NAME.to_owned(),
            code: error.code,
            source: Box::new(MessageError(error.message)),
        },
    }
}

pub(super) fn unavailable_error(resource: impl Into<String>, reason: impl Into<String>) -> VmError {
    VmError::Unavailable {
        resource: resource.into(),
        reason: reason.into(),
    }
}

pub(super) fn provider_error(
    code: &'static str,
    source: impl Error + Send + Sync + 'static,
) -> VmError {
    VmError::Provider {
        provider: PROVIDER_NAME.to_owned(),
        code: code.to_owned(),
        source: Box::new(source),
    }
}

#[derive(Debug)]
struct MessageError(String);

impl fmt::Display for MessageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for MessageError {}
