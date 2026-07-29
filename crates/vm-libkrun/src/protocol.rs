//! Versioned host-to-guest protocol spoken with `heph-init`.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Current host-to-guest protocol version.
pub const PROTOCOL_VERSION: u16 = 3;

/// `AF_VSOCK` port used by `heph-init` to connect to the host worker.
pub const GUEST_VSOCK_PORT: u32 = 19_000;
/// Dedicated guest-to-host secret broker port.
pub const SECRET_BROKER_VSOCK_PORT: u32 = 19_001;

/// Maximum encoded protocol frame size.
pub const MAX_FRAME_SIZE: usize = 16 * 1024 * 1024;

/// Maximum bytes carried by one log message.
pub const MAX_LOG_CHUNK_SIZE: usize = 64 * 1024;

/// Maximum UTF-8 bytes in a metric name or label component.
pub const MAX_METRIC_TEXT_SIZE: usize = 256;

/// Maximum labels carried by one metric.
pub const MAX_METRIC_LABELS: usize = 64;

/// Maximum UTF-8 bytes in a result finalization message.
pub const MAX_RESULT_MESSAGE_SIZE: usize = 4 * 1024;

/// A command sent from the host worker to `heph-init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum HostMessage {
    /// Starts the approved guest command.
    Start {
        /// Protocol version expected by the host.
        version: u16,
        /// Command to execute without a shell.
        command: GuestCommandMessage,
        /// Filesystems `heph-init` must mount before executing the command.
        mounts: Vec<GuestMount>,
        /// Persistent agent-state volume to locate by filesystem UUID.
        state_volume: Option<GuestStateVolume>,
    },
    /// Requests graceful cancellation.
    Cancel {
        /// Maximum grace period before host-side forced termination.
        timeout_ms: u64,
    },
    /// Checks guest control-channel liveness.
    HealthPing {
        /// Opaque value echoed by the guest.
        nonce: u64,
    },
}

/// A message sent by `heph-init` to the host worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GuestMessage {
    /// Announces the guest protocol version after connecting.
    Hello {
        /// Protocol version implemented by the guest.
        version: u16,
    },
    /// Reports that the command and mounts were accepted.
    Ready,
    /// Carries an uninterpreted command output chunk.
    Log {
        /// Output stream that produced the chunk.
        stream: GuestLogStream,
        /// Raw output bytes.
        bytes: Vec<u8>,
    },
    /// Carries a structured guest metric.
    Metric {
        /// Stable metric name.
        name: String,
        /// Numeric metric value.
        value: f64,
        /// Metric dimensions.
        labels: BTreeMap<String, String>,
    },
    /// Responds to a health check.
    Health {
        /// Opaque value from the corresponding ping.
        nonce: u64,
    },
    /// Declares that the guest is done modifying its writable workspace.
    FinalizeResult {
        /// Human-readable result commit message.
        message: String,
    },
    /// Reports the final command status.
    Exited {
        /// Exit code for a normal exit.
        code: Option<i32>,
        /// Signal number for a signal-based exit.
        signal: Option<i32>,
    },
    /// Reports a guest bootstrap or command failure.
    Error {
        /// Stable guest-side diagnostic code.
        code: String,
        /// Human-readable diagnostic.
        message: String,
    },
}

/// Command representation transmitted to `heph-init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestCommandMessage {
    /// Absolute program path inside the guest.
    pub program: String,
    /// Arguments excluding the program itself.
    pub args: Vec<String>,
    /// Complete command environment.
    pub env: BTreeMap<String, String>,
    /// Optional absolute working directory inside the guest.
    pub working_dir: Option<PathBuf>,
}

/// A virtio-fs mount to attach inside the guest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestMount {
    /// Virtio-fs device tag.
    pub tag: String,
    /// Absolute destination path inside the guest.
    pub guest_path: PathBuf,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

/// Persistent state volume mounted by `heph-init`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuestStateVolume {
    /// Stable ext4 filesystem UUID.
    pub filesystem_uuid: String,
    /// Absolute guest mount point.
    pub guest_path: PathBuf,
}

/// Guest output stream identifier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[non_exhaustive]
pub enum GuestLogStream {
    /// Standard output.
    Stdout,
    /// Standard error.
    Stderr,
}
