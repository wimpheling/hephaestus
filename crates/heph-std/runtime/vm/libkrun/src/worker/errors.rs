use super::types::{WireError, WireErrorKind, WireLogStream};
use crate::{ffi::FfiError, network::WorkerNetworkError, protocol::GuestLogStream};
use std::io;

impl WireError {
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self {
            kind: WireErrorKind::InvalidState,
            code: "invalid-state".to_owned(),
            message: message.into(),
        }
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self {
            kind: WireErrorKind::Unsupported,
            code: "unsupported".to_owned(),
            message: message.into(),
        }
    }

    pub(super) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            kind: WireErrorKind::Unavailable,
            code: "unavailable".to_owned(),
            message: message.into(),
        }
    }

    pub(super) fn io(error: &io::Error) -> Self {
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
pub(super) enum WorkerError {
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
