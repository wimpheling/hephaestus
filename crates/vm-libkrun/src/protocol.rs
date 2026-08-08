//! Versioned host-to-guest protocol spoken with `heph-init`.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

/// Current host-to-guest protocol version.
pub const PROTOCOL_VERSION: u16 = 5;

/// `AF_VSOCK` port used by `heph-init` to connect to the host worker.
pub const GUEST_VSOCK_PORT: u32 = 19_000;
/// Dedicated guest-to-host secret broker port.
pub const SECRET_BROKER_VSOCK_PORT: u32 = 19_001;
/// Guest-private file populated from the authenticated runtime-authority
/// bootstrap payload before the workload starts.
pub const GUEST_RUNTIME_AUTHORITY_PATH: &str = "/run/hephaestus-authority/session.json";

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
        /// Sensitive one-run authority delivered only on this authenticated
        /// host-to-guest bootstrap stream.
        runtime_authority: Option<Box<RuntimeAuthorityMessage>>,
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
    /// Confirms that guest bootstrap persisted the exact authority payload.
    RuntimeAuthorityAcknowledged {
        /// Exact runtime session identifier.
        session_id: uuid::Uuid,
        /// Exact issuance generation received by the guest.
        generation: u64,
    },
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

/// Sensitive runtime authority carried only by the bootstrap stream.
#[derive(Clone, Serialize, Deserialize)]
pub struct RuntimeAuthorityMessage {
    /// Exact runtime session identifier.
    pub session_id: uuid::Uuid,
    /// Exact positive issuance generation.
    pub generation: u64,
    /// Opaque bearer bytes.
    pub credential: [u8; vm_trait::RUNTIME_AUTHORITY_CREDENTIAL_BYTES],
    /// Separate exact-run Git bearer, when runtime Git is bound.
    pub runtime_git_credential: Option<[u8; vm_trait::RUNTIME_GIT_CREDENTIAL_BYTES]>,
}

impl std::fmt::Debug for RuntimeAuthorityMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAuthorityMessage")
            .field("session_id", &self.session_id)
            .field("generation", &self.generation)
            .field("credential", &"[REDACTED]")
            .field(
                "runtime_git_credential",
                &self.runtime_git_credential.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl Drop for RuntimeAuthorityMessage {
    fn drop(&mut self) {
        for byte in &mut self.credential {
            *std::hint::black_box(byte) = 0;
        }
        if let Some(credential) = &mut self.runtime_git_credential {
            for byte in credential {
                *std::hint::black_box(byte) = 0;
            }
        }
    }
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
