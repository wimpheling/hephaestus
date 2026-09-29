use bytes::Bytes;
use http::{HeaderMap, Method, StatusCode};
use tokio::io::{AsyncRead, AsyncWrite};

/// Provider-neutral full-duplex stream for a private HTTP service connection.
pub trait PrivateServiceConnection: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T> PrivateServiceConnection for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

/// Owned provider-neutral private service stream.
pub type BoxedPrivateServiceConnection = Box<dyn PrivateServiceConnection>;

/// One complete host-to-guest request on the VM's private control transport.
///
/// This is not a guest network listener, a port forward, or a public socket.
/// The provider carries it only after it has started an exact VM through its
/// authenticated private host/guest channel.  Bodies are intentionally
/// complete in memory: streaming, trailers, upgrades, and `WebSockets` are not
/// part of this operation.
#[derive(Debug, Clone)]
pub struct PrivateHttpRequest {
    /// Canonical HTTP method selected by the trusted host dispatcher.
    pub method: Method,
    /// Normalized absolute path and optional query.
    pub path_and_query: String,
    /// Bounded canonical request headers.
    pub headers: HeaderMap,
    /// Complete bounded request body.
    pub body: Bytes,
}

/// One complete guest-to-host response on the VM's private control transport.
#[derive(Debug, Clone)]
pub struct PrivateHttpResponse {
    /// HTTP status selected by guest application code.
    pub status: StatusCode,
    /// Bounded canonical response headers.
    pub headers: HeaderMap,
    /// Complete bounded response body.
    pub body: Bytes,
    /// Optional, bounded request to publish one generic event through a
    /// mailbox capability already bound by the trusted host.
    ///
    /// This is data on the existing private response channel, not a guest
    /// network connection or a mailbox credential.  The host fixes the target
    /// mailbox and producer from `slot` before it authorizes acceptance.
    pub mailbox_publication: Option<PrivateMailboxPublication>,
}

/// One candidate generic mailbox publication returned by an `http.v1` guest.
///
/// The fields deliberately preserve only normalized request-like data.  The
/// guest cannot select a target mailbox or producer identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateMailboxPublication {
    /// Exact immutable capability slot selected by the released gateway.
    pub slot: String,
    /// Uppercase normalized application method.
    pub method: String,
    /// Normalized absolute application route.
    pub route: String,
    /// Bounded application-selected metadata, not HTTP forwarding headers.
    pub headers: Vec<(String, String)>,
    /// Optional bounded media type selected by the application.
    pub content_type: Option<String>,
    /// Optional bounded application trace context.
    pub trace_context: Option<String>,
    /// Complete bounded event body.
    pub body: Bytes,
    /// Stable application-provided idempotency key.
    pub deduplication_key: String,
}
