use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf};

use super::constants::PRIVATE_SERVICE_CHALLENGE_BYTES;

/// A single-use private service handshake challenge.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct PrivateServiceChallenge(pub [u8; PRIVATE_SERVICE_CHALLENGE_BYTES]);

impl PrivateServiceChallenge {
    /// Returns the challenge bytes for the fixed handshake encoding.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; PRIVATE_SERVICE_CHALLENGE_BYTES] {
        &self.0
    }
}

impl std::fmt::Debug for PrivateServiceChallenge {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

/// Host-to-guest metadata authorizing one private service connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateServiceConnectionMessage {
    /// Exact pending connection identifier.
    pub connection_id: uuid::Uuid,
    /// Single-use proof bound to `connection_id`.
    pub challenge: PrivateServiceChallenge,
}

/// Wire representation of a declared long-lived private HTTP service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateHttpServiceMessage {
    /// Guest loopback TCP port on which the released server listens.
    pub loopback_port: u16,
    /// Maximum number of provider-managed service connections in flight.
    pub max_connections: u32,
    /// Maximum time allowed to connect to the guest loopback server.
    pub connect_timeout_ms: u64,
}

/// Wire metadata for the guest-local runtime-Git proxy. It contains no
/// repository authority or bearer material.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct RuntimeGitBridgeMessage {
    /// Repository UUID whose route and credential are authorized for this VM.
    pub repository_id: uuid::Uuid,
    /// Guest loopback port on which `heph-init` accepts Git HTTP bytes.
    pub loopback_port: u16,
}

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
        /// Whether `command` is a one-request private HTTP gateway handler.
        gateway_handler: bool,
        /// Optional long-lived private HTTP service declaration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        private_http_service: Option<PrivateHttpServiceMessage>,
        /// Optional exact-run guest-to-host runtime-Git proxy declaration.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        runtime_git_bridge: Option<RuntimeGitBridgeMessage>,
    },
    /// Opens one authorized guest-initiated private service connection.
    OpenPrivateServiceConnection {
        /// Exact connection authorization metadata.
        connection: PrivateServiceConnectionMessage,
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
    /// Invokes one gateway handler request over the authenticated control channel.
    PrivateHttpRequest {
        /// Correlates the exact guest response.
        request_id: u64,
        /// Complete bounded request.
        request: PrivateHttpRequestMessage,
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
    /// Returns exactly one bounded gateway handler response.
    PrivateHttpResponse {
        /// Correlates the exact host request.
        request_id: u64,
        /// Complete bounded response.
        response: PrivateHttpResponseMessage,
    },
}

/// Wire representation of a complete canonical private HTTP request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateHttpRequestMessage {
    /// Uppercase canonical HTTP method.
    pub method: String,
    /// Normalized absolute path and optional query.
    pub path_and_query: String,
    /// Bounded canonical header name/value pairs.
    pub headers: Vec<(String, String)>,
    /// Complete bounded body.
    pub body: Vec<u8>,
}

/// Wire representation of a complete canonical private HTTP response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateHttpResponseMessage {
    /// Three-digit HTTP status.
    pub status: u16,
    /// Bounded canonical header name/value pairs.
    pub headers: Vec<(String, String)>,
    /// Bounded complete response body.
    pub body: Vec<u8>,
    /// At most one generic mailbox publication candidate.
    pub mailbox_publication: Option<PrivateMailboxPublicationMessage>,
}

/// Wire representation of an application-selected mailbox publication.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateMailboxPublicationMessage {
    /// Exact bound capability slot; target and producer remain host-selected.
    pub slot: String,
    /// Uppercase normalized application method.
    pub method: String,
    /// Normalized absolute application route.
    pub route: String,
    /// Bounded application-selected metadata.
    pub headers: Vec<(String, String)>,
    /// Optional application media type.
    pub content_type: Option<String>,
    /// Optional application trace context.
    pub trace_context: Option<String>,
    /// Complete bounded event body.
    pub body: Vec<u8>,
    /// Stable application-provided idempotency key.
    pub deduplication_key: String,
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
