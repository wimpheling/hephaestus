use crate::{
    protocol::PrivateServiceConnectionMessage,
    validation::{PreparedForward, PreparedSpec},
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, ffi::OsString, path::PathBuf, time::Duration};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerConfiguration {
    pub passt_binary: PathBuf,
    pub libkrun_library: OsString,
    pub service_uid: u32,
    pub service_gid: u32,
    pub startup_timeout: Duration,
    pub broker_socket_path: Option<PathBuf>,
    pub runtime_git_socket_path: Option<PathBuf>,
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
    InvokePrivateHttp {
        request_id: u64,
        request: crate::protocol::PrivateHttpRequestMessage,
    },
    OpenPrivateServiceConnection {
        connection: PrivateServiceConnectionMessage,
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
    RuntimeAuthorityAcknowledged {
        session_id: uuid::Uuid,
        generation: u64,
    },
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
    /// A bounded private HTTP response from the gateway guest handler.
    PrivateHttpResponse {
        /// Correlates the exact host request.
        request_id: u64,
        /// Bounded canonical response.
        response: crate::protocol::PrivateHttpResponseMessage,
    },
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
