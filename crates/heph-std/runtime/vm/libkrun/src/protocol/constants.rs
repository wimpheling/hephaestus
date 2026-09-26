/// Current host-to-guest protocol version.
pub const PROTOCOL_VERSION: u16 = 9;
/// Maximum private HTTP body carried by the authenticated control protocol.
pub const MAX_PRIVATE_HTTP_BODY_BYTES: usize = 1_048_576;
/// Maximum private HTTP headers carried by one request or response.
pub const MAX_PRIVATE_HTTP_HEADERS: usize = 64;
/// Maximum bytes in one generic mailbox publication body.
pub const MAX_MAILBOX_PUBLICATION_BODY_BYTES: usize = 1_048_576;
/// Maximum selected metadata entries in one mailbox publication.
pub const MAX_MAILBOX_PUBLICATION_HEADERS: usize = 32;
/// Maximum UTF-8 bytes in a mailbox capability slot.
pub const MAX_MAILBOX_PUBLICATION_SLOT_BYTES: usize = 64;
/// Maximum UTF-8 bytes in a generic publication method.
pub const MAX_MAILBOX_PUBLICATION_METHOD_BYTES: usize = 16;
/// Maximum UTF-8 bytes in a generic publication route.
pub const MAX_MAILBOX_PUBLICATION_ROUTE_BYTES: usize = 1_024;
/// Maximum UTF-8 bytes in one generic publication metadata name.
pub const MAX_MAILBOX_PUBLICATION_HEADER_NAME_BYTES: usize = 64;
/// Maximum UTF-8 bytes in a generic publication metadata value.
pub const MAX_MAILBOX_PUBLICATION_HEADER_VALUE_BYTES: usize = 1_024;
/// Maximum UTF-8 bytes in a generic publication content type.
pub const MAX_MAILBOX_PUBLICATION_CONTENT_TYPE_BYTES: usize = 256;
/// Maximum UTF-8 bytes in a generic publication trace context.
pub const MAX_MAILBOX_PUBLICATION_TRACE_CONTEXT_BYTES: usize = 512;
/// Maximum UTF-8 bytes in a stable publication idempotency key.
pub const MAX_MAILBOX_PUBLICATION_DEDUPLICATION_KEY_BYTES: usize = 256;
/// Maximum encoded output collected from a one-shot gateway handler.
pub const MAX_GATEWAY_HANDLER_OUTPUT_BYTES: usize =
    MAX_PRIVATE_HTTP_BODY_BYTES + MAX_MAILBOX_PUBLICATION_BODY_BYTES + 131_072;
/// VM label which opts a released command into the one-request gateway ABI.
///
/// This is deliberately an exact contract value rather than a generic
/// networking switch: ordinary VM commands must never receive control-plane
/// HTTP frames on their standard input.
pub const GATEWAY_HANDLER_CONTRACT_LABEL: &str = "hephaestus.gateway.handler-contract";
/// The only gateway handler contract understood by this protocol version.
pub const GATEWAY_HANDLER_CONTRACT_V1: &str = "http.v1";

/// Fixed vsock port used for the guest-initiated private service bridge.
pub const PRIVATE_SERVICE_VSOCK_PORT: u32 = 19_002;
/// Stable magic prefix for the private service handshake.
pub const PRIVATE_SERVICE_HANDSHAKE_MAGIC: [u8; 8] = *b"HEPH-SVC";
/// Current private service handshake version.
pub const PRIVATE_SERVICE_HANDSHAKE_VERSION: u8 = 1;
/// Number of unpredictable bytes in one private service challenge.
pub const PRIVATE_SERVICE_CHALLENGE_BYTES: usize = 32;
/// Parent-owned Unix socket mapped to [`PRIVATE_SERVICE_VSOCK_PORT`].
pub const PRIVATE_SERVICE_SOCKET_NAME: &str = "private-service.sock";

/// Dedicated guest-to-host runtime-Git bridge port. It is separate from the
/// secret broker and the host-to-guest private service transport.
pub const RUNTIME_GIT_VSOCK_PORT: u32 = 19_003;
/// Default guest loopback port for the token-free runtime-Git remote.
pub const RUNTIME_GIT_LOOPBACK_PORT: u16 = 19_100;
/// Guest helper installed in approved images and selected through Git config
/// environment variables, never by embedding a credential in configuration.
pub const RUNTIME_GIT_CREDENTIAL_HELPER: &str = "/usr/libexec/hephaestus/heph-git-credential";
/// Guest-local environment variable naming the expected proxy host.
pub const RUNTIME_GIT_HOST_ENV: &str = "HEPH_RUNTIME_GIT_HOST";
/// Guest-local environment variable naming the exact repository route.
pub const RUNTIME_GIT_PATH_ENV: &str = "HEPH_RUNTIME_GIT_PATH";

/// `AF_VSOCK` port used by `heph-init` to connect to the host worker.
pub const GUEST_VSOCK_PORT: u32 = 19_000;
/// Dedicated guest-to-host secret broker port.
pub const SECRET_BROKER_VSOCK_PORT: u32 = 19_001;
/// Private provider-worker socket used to supervise one VM worker.
pub const SUPERVISOR_SOCKET_NAME: &str = "supervisor.sock";
/// Private worker socket passed to libkrun for guest control.
pub const GUEST_CONTROL_SOCKET_NAME: &str = "guest.sock";
/// Guest-private file populated from the authenticated runtime-authority
/// bootstrap payload before the workload starts.
pub const GUEST_RUNTIME_AUTHORITY_PATH: &str = "/run/hephaestus-authority/session.json";
/// Guest environment variable naming this invocation's authority credential.
pub const RUNTIME_AUTHORITY_PATH_ENV: &str = "HEPH_RUNTIME_AUTHORITY_PATH";

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
