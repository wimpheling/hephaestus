//! Bounded semantic secret-broker protocol over a host Unix socket that
//! libkrun exposes through a dedicated vsock port.

use async_trait::async_trait;
use brokered_egress_domain::{
    BrokeredEgressError, BrokeredSecretRule, HttpInjectionLocation, InjectionDirection,
};
use runtime_types::RunId;
use secret_application::{
    BrokerAdapter, BrokerAdapterError, BrokerRequest, BrokerResponse, BrokerStatus,
    SecretRuntimeResolver,
};
use secret_domain::{OpaqueRuntimeCredential, SecretSlotKey, SecretValue};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    net::SocketAddr,
    os::unix::fs::{FileTypeExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, UnixListener, UnixStream},
    sync::Semaphore,
};
use tokio_util::sync::CancellationToken;

const MAX_FRAME_BYTES: usize = 1024 * 1024;
const MAX_CREDENTIAL_BYTES: usize = 256;
const MAX_ADAPTER_BODY_BYTES: usize = 64 * 1024;
const MAX_UPSTREAM_RESPONSE_BYTES: u64 = 64 * 1024;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_HTTPS_HEADERS: usize = 32;
const MAX_HTTPS_PATH_BYTES: usize = 4_096;

/// Guest-provided HTTP request transported over the broker vsock channel.
///
/// This is the locked-plan cooperating transport: the guest has no IP
/// interface in `BrokerOnly` mode, so it cannot bypass this host-side TLS
/// client or observe the substituted credential.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsRequest {
    /// Immutable rule claimed by this request. The runtime resolver verifies
    /// it against the exact lease snapshot before it decrypts a secret.
    pub rule_id: uuid::Uuid,
    /// Bounded HTTP method. The MVP keeps this intentionally small.
    pub method: BrokeredHttpsMethod,
    /// Origin-relative path and optional query, never an absolute URL.
    pub path_and_query: String,
    /// Request headers. Header names are lower-case ASCII.
    pub headers: Vec<BrokeredHttpsHeader>,
    /// Bounded request body.
    pub body: Vec<u8>,
}

/// Supported ordinary HTTPS methods.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokeredHttpsMethod {
    /// Read an endpoint.
    Get,
    /// Submit a bounded request.
    Post,
    /// Replace a bounded endpoint value.
    Put,
    /// Remove an endpoint value.
    Delete,
}

/// One request header, free of credentials until the host applies its rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokeredHttpsHeader {
    /// Lower-case ASCII header name.
    pub name: String,
    /// Header value with no line breaks or NUL bytes.
    pub value: String,
}

/// Host-side request after exact placeholder substitution.
///
/// This type intentionally has no `Debug` implementation because one header
/// may contain plaintext resolved only inside the secret authority boundary.
pub struct UpstreamHttpsRequest {
    method: BrokeredHttpsMethod,
    destination: String,
    path_and_query: String,
    headers: Vec<BrokeredHttpsHeader>,
    body: Vec<u8>,
}

impl UpstreamHttpsRequest {
    /// Returns the fixed destination for a trusted host transport.
    #[must_use]
    pub fn destination(&self) -> &str {
        &self.destination
    }

    /// Returns the validated relative request path.
    #[must_use]
    pub fn path_and_query(&self) -> &str {
        &self.path_and_query
    }
}

/// Host-only HTTPS transport. Implementations must make a verified TLS
/// connection to the exact `destination` and must not follow redirects.
#[async_trait]
pub trait PinnedHttpsTransport: Send + Sync + 'static {
    /// Sends one already-authorized and substituted request.
    async fn send(
        &self,
        request: UpstreamHttpsRequest,
    ) -> Result<BrokerResponse, BrokerAdapterError>;
}

/// Generic adapter that turns an ordinary bounded HTTPS request into one
/// brokered secret use without provider-specific semantics.
pub struct BrokeredHttpsAdapter<T> {
    destination: String,
    rule: BrokeredSecretRule,
    transport: T,
}

impl<T> BrokeredHttpsAdapter<T> {
    /// Creates an adapter from one immutable outbound authority rule.
    ///
    /// # Errors
    ///
    /// Rejects inbound rules, broad destinations, and non-default HTTPS ports.
    /// The legacy runtime secret ceiling is a DNS host, not a URL, so explicit
    /// ports are deliberately deferred until that authority representation is
    /// extended consistently.
    pub fn new(rule: BrokeredSecretRule, transport: T) -> Result<Self, BrokerAdapterError> {
        let rule = rule.normalized().map_err(map_rule_error)?;
        if rule.location.direction() != InjectionDirection::Outbound {
            return Err(BrokerAdapterError::Rejected);
        }
        let origin = rule
            .destination
            .as_ref()
            .ok_or(BrokerAdapterError::Rejected)?
            .as_str();
        let destination = origin
            .strip_prefix("https://")
            .ok_or(BrokerAdapterError::Rejected)?;
        if destination.contains(':') || !valid_dns_destination(destination) {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(Self {
            destination: destination.to_owned(),
            rule,
            transport,
        })
    }
}

#[async_trait]
impl<T> BrokerAdapter for BrokeredHttpsAdapter<T>
where
    T: PinnedHttpsTransport,
{
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        if destination != self.destination
            || operation != "https_v1"
            || body.len() > MAX_ADAPTER_BODY_BYTES
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let request: BrokeredHttpsRequest =
            serde_json::from_slice(body).map_err(|_| BrokerAdapterError::Rejected)?;
        if request.rule_id != self.rule.id.as_uuid() {
            return Err(BrokerAdapterError::Rejected);
        }
        let headers = validate_and_substitute_headers(&request, &self.rule, credential.expose())?;
        let response = self
            .transport
            .send(UpstreamHttpsRequest {
                method: request.method,
                destination: self.destination.clone(),
                path_and_query: validate_https_path(&request.path_and_query)?,
                headers,
                body: request.body,
            })
            .await?;
        // An allowed upstream is still untrusted with respect to the guest:
        // it could reflect an authorization header in its body.  Never turn a
        // host-only substitution into plaintext delivery through a response.
        if response
            .body
            .windows(credential.expose().len())
            .any(|window| window == credential.expose())
        {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(response)
    }
}

/// Default redirect-free, proxy-free TLS transport.
///
/// It uses deployment-pinned DNS addresses. `reqwest` verifies the upstream
/// certificate and SNI against the exact DNS destination; no
/// invalid-certificate escape hatch is exposed.
pub struct ReqwestPinnedHttpsTransport {
    client: reqwest::Client,
}

impl ReqwestPinnedHttpsTransport {
    /// Builds a TLS client pinned to public resolved addresses for one host.
    ///
    /// The caller obtains and rotates these addresses through trusted control
    /// plane DNS resolution. Pinning prevents a guest-selected DNS rebinding.
    ///
    /// # Errors
    ///
    /// Rejects invalid destinations, empty or non-public address pins, and
    /// TLS client construction failures.
    pub fn new(
        destination: &str,
        addresses: &[std::net::IpAddr],
    ) -> Result<Self, BrokerAdapterError> {
        if !valid_dns_destination(destination)
            || addresses.is_empty()
            || addresses.iter().any(|address| !public_address(*address))
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let addresses = addresses
            .iter()
            .copied()
            .map(|address| SocketAddr::new(address, 443))
            .collect::<Vec<_>>();
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(UPSTREAM_TIMEOUT)
            .timeout(UPSTREAM_TIMEOUT)
            .resolve_to_addrs(destination, &addresses)
            .build()
            .map_err(|_| BrokerAdapterError::Rejected)?;
        Ok(Self { client })
    }
}

#[async_trait]
impl PinnedHttpsTransport for ReqwestPinnedHttpsTransport {
    async fn send(
        &self,
        request: UpstreamHttpsRequest,
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        let url = format!("https://{}{}", request.destination, request.path_and_query);
        let method = match request.method {
            BrokeredHttpsMethod::Get => reqwest::Method::GET,
            BrokeredHttpsMethod::Post => reqwest::Method::POST,
            BrokeredHttpsMethod::Put => reqwest::Method::PUT,
            BrokeredHttpsMethod::Delete => reqwest::Method::DELETE,
        };
        let mut headers = reqwest::header::HeaderMap::new();
        for header in request.headers {
            let name = reqwest::header::HeaderName::from_bytes(header.name.as_bytes())
                .map_err(|_| BrokerAdapterError::Rejected)?;
            let value = reqwest::header::HeaderValue::from_str(&header.value)
                .map_err(|_| BrokerAdapterError::Rejected)?;
            headers.append(name, value);
        }
        let response = self
            .client
            .request(method, url)
            .headers(headers)
            .body(request.body)
            .send()
            .await
            .map_err(|_| BrokerAdapterError::Retryable)?;
        let status = response.status();
        if status.is_redirection() {
            return Err(BrokerAdapterError::Rejected);
        }
        let body = response
            .bytes()
            .await
            .map_err(|_| BrokerAdapterError::Retryable)?;
        if body.len()
            > usize::try_from(MAX_UPSTREAM_RESPONSE_BYTES).expect("response bound fits usize")
        {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(BrokerResponse {
            status: if status.is_server_error() {
                BrokerStatus::Retryable
            } else if status.is_success() {
                BrokerStatus::Succeeded
            } else {
                BrokerStatus::Rejected
            },
            body: body.to_vec(),
        })
    }
}

fn validate_and_substitute_headers(
    request: &BrokeredHttpsRequest,
    rule: &BrokeredSecretRule,
    credential: &[u8],
) -> Result<Vec<BrokeredHttpsHeader>, BrokerAdapterError> {
    if request.headers.len() > MAX_HTTPS_HEADERS || request.body.len() > MAX_ADAPTER_BODY_BYTES {
        return Err(BrokerAdapterError::Rejected);
    }
    let mut seen = std::collections::HashSet::new();
    let mut substitutions = 0_u8;
    let mut headers = Vec::with_capacity(request.headers.len());
    let (rule_header, value_prefix) = match &rule.location {
        HttpInjectionLocation::OutboundHeaderValue { header } => (header.as_str(), ""),
        HttpInjectionLocation::OutboundHeaderPrefix { header, prefix } => {
            (header.as_str(), prefix.as_str())
        }
        HttpInjectionLocation::InboundGatewayHeader { .. } => {
            return Err(BrokerAdapterError::Rejected);
        }
    };
    let placeholder = rule.placeholder();
    for header in &request.headers {
        if !valid_header_name(&header.name)
            || header.name == "host"
            || header.name == "proxy-authorization"
            || contains_http_control(&header.value)
            || !seen.insert(header.name.as_str())
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let value = if header.name == rule_header {
            let expected = format!("{value_prefix}{placeholder}");
            if header.value != expected || credential.is_empty() {
                return Err(BrokerAdapterError::Rejected);
            }
            substitutions = substitutions.saturating_add(1);
            let mut replaced = String::with_capacity(value_prefix.len() + credential.len());
            replaced.push_str(value_prefix);
            let suffix =
                std::str::from_utf8(credential).map_err(|_| BrokerAdapterError::Rejected)?;
            replaced.push_str(suffix);
            replaced
        } else {
            header.value.clone()
        };
        headers.push(BrokeredHttpsHeader {
            name: header.name.clone(),
            value,
        });
    }
    if substitutions != 1 {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(headers)
}

const fn map_rule_error(_error: BrokeredEgressError) -> BrokerAdapterError {
    BrokerAdapterError::Rejected
}

fn validate_https_path(value: &str) -> Result<String, BrokerAdapterError> {
    if !(1..=MAX_HTTPS_PATH_BYTES).contains(&value.len())
        || !value.starts_with('/')
        || value.starts_with("//")
        || value.contains('#')
        || contains_http_control(value)
    {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(value.to_owned())
}

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn contains_http_control(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| matches!(byte, b'\r' | b'\n' | b'\0'))
}

const fn public_address(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            !address.is_private()
                && !address.is_loopback()
                && !address.is_link_local()
                && !address.is_broadcast()
                && !address.is_unspecified()
                && !address.is_multicast()
                && address.octets()[0] != 0
        }
        std::net::IpAddr::V6(address) => {
            !address.is_loopback()
                && !address.is_unspecified()
                && !address.is_multicast()
                && !address.is_unicast_link_local()
                && !address.is_unique_local()
        }
    }
}

/// One guest request. The credential is deliberately omitted from formatting
/// traits so accidental debug logging cannot disclose it.
#[derive(Deserialize, Serialize)]
pub struct WireBrokerRequest {
    /// Short-lived credential read from the ephemeral runtime mount.
    pub credential: Vec<u8>,
    /// Exact claimed run.
    pub run_id: RunId,
    /// Symbolic release slot.
    pub slot: String,
    /// Exact allowlisted destination name.
    pub destination: String,
    /// Semantic adapter operation.
    pub operation: String,
    /// Bounded application body.
    pub body: Vec<u8>,
}

/// Sanitized provider-neutral response.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct WireBrokerResponse {
    /// Stable result class.
    pub status: WireBrokerStatus,
    /// Adapter-sanitized body.
    pub body: Vec<u8>,
}

/// Stable wire result class without internal diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WireBrokerStatus {
    /// Semantic operation completed.
    Succeeded,
    /// Authority or operation was denied.
    Denied,
    /// A retry may succeed.
    Retryable,
}

/// Host-side semantic broker execution boundary.
#[async_trait]
pub trait BrokerExecutor: Send + Sync + 'static {
    /// Authenticates and executes one bounded request.
    async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse;
}

/// Connects the wire protocol to live secret runtime authorization.
pub struct ServiceBrokerExecutor {
    runtime: Arc<dyn SecretRuntimeResolver>,
    adapter: Arc<dyn BrokerAdapter>,
}

impl ServiceBrokerExecutor {
    /// Creates the live executor.
    #[must_use]
    pub fn new(runtime: Arc<dyn SecretRuntimeResolver>, adapter: Arc<dyn BrokerAdapter>) -> Self {
        Self { runtime, adapter }
    }
}

#[async_trait]
impl BrokerExecutor for ServiceBrokerExecutor {
    async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse {
        if request.credential.len() > MAX_CREDENTIAL_BYTES {
            return denied();
        }
        let Ok(credential) = OpaqueRuntimeCredential::new(request.credential) else {
            return denied();
        };
        let Ok(slot) = SecretSlotKey::parse(request.slot) else {
            return denied();
        };
        let semantic = BrokerRequest {
            run_id: request.run_id,
            slot,
            destination: request.destination,
            operation: request.operation,
            body: request.body,
        };
        match self
            .runtime
            .use_brokered(&credential, &semantic, self.adapter.as_ref())
            .await
        {
            Ok(BrokerResponse { status, body }) => WireBrokerResponse {
                status: match status {
                    BrokerStatus::Succeeded => WireBrokerStatus::Succeeded,
                    BrokerStatus::Rejected => WireBrokerStatus::Denied,
                    BrokerStatus::Retryable => WireBrokerStatus::Retryable,
                },
                body,
            },
            Err(error) => {
                tracing::warn!(
                    run_id = %semantic.run_id,
                    slot = %semantic.slot,
                    error_class = error_class(&error),
                    "secret broker request failed"
                );
                denied()
            }
        }
    }
}

/// Production-safe default adapter used when no semantic provider adapter is
/// configured.
pub struct DenyingBrokerAdapter;

#[async_trait]
impl BrokerAdapter for DenyingBrokerAdapter {
    async fn invoke(
        &self,
        _credential: &SecretValue,
        _destination: &str,
        _operation: &str,
        _body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        Err(BrokerAdapterError::Rejected)
    }
}

/// Narrow loopback-only bearer adapter for the initial fake-upstream proof.
///
/// The guest chooses neither an address nor a path. A trusted host pins the
/// loopback listener and logical DNS destination, and the only operation is
/// `complete`. The adapter never follows redirects or forwards response
/// headers and returns only a bounded `result` JSON field.
pub struct LoopbackCompletionAdapter {
    destination: String,
    upstream: SocketAddr,
    concurrency: Arc<Semaphore>,
}

impl LoopbackCompletionAdapter {
    /// Creates a rate-limited adapter for one trusted fake upstream.
    ///
    /// # Errors
    ///
    /// Rejects non-loopback endpoints, unsafe logical destinations, and a
    /// zero concurrency ceiling.
    pub fn new(
        destination: impl Into<String>,
        upstream: SocketAddr,
        max_in_flight: usize,
    ) -> Result<Self, BrokerAdapterError> {
        let destination = destination.into();
        if !valid_dns_destination(&destination)
            || !upstream.ip().is_loopback()
            || max_in_flight == 0
        {
            return Err(BrokerAdapterError::Rejected);
        }
        Ok(Self {
            destination,
            upstream,
            concurrency: Arc::new(Semaphore::new(max_in_flight)),
        })
    }
}

#[async_trait]
impl BrokerAdapter for LoopbackCompletionAdapter {
    async fn invoke(
        &self,
        credential: &SecretValue,
        destination: &str,
        operation: &str,
        body: &[u8],
    ) -> Result<BrokerResponse, BrokerAdapterError> {
        if destination != self.destination
            || operation != "complete"
            || body.len() > MAX_ADAPTER_BODY_BYTES
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let credential = credential.expose();
        if credential.is_empty()
            || credential.len() > MAX_CREDENTIAL_BYTES
            || !credential.iter().copied().all(valid_bearer_byte)
        {
            return Err(BrokerAdapterError::Rejected);
        }
        let _permit = Arc::clone(&self.concurrency)
            .try_acquire_owned()
            .map_err(|_| BrokerAdapterError::Retryable)?;
        let response = tokio::time::timeout(
            UPSTREAM_TIMEOUT,
            invoke_loopback(self.upstream, &self.destination, credential, body),
        )
        .await
        .map_err(|_| BrokerAdapterError::Retryable)??;
        sanitize_upstream_response(&response, credential)
    }
}

async fn invoke_loopback(
    upstream: SocketAddr,
    destination: &str,
    credential: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, BrokerAdapterError> {
    let mut stream = TcpStream::connect(upstream)
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    let header = format!(
        "POST /v1/complete HTTP/1.1\r\nHost: {destination}\r\n\
         Authorization: Bearer "
    );
    stream
        .write_all(header.as_bytes())
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    stream
        .write_all(credential)
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    let suffix = format!(
        "\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
         Connection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(suffix.as_bytes())
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    stream
        .write_all(body)
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    stream
        .shutdown()
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    let mut response = Vec::new();
    stream
        .take(MAX_UPSTREAM_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .await
        .map_err(|_| BrokerAdapterError::Retryable)?;
    if u64::try_from(response.len()).unwrap_or(u64::MAX) > MAX_UPSTREAM_RESPONSE_BYTES {
        return Err(BrokerAdapterError::Rejected);
    }
    Ok(response)
}

fn sanitize_upstream_response(
    response: &[u8],
    credential: &[u8],
) -> Result<BrokerResponse, BrokerAdapterError> {
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(BrokerAdapterError::Rejected)?;
    let headers = &response[..header_end];
    let body = &response[header_end + 4..];
    let status_line = headers
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or(BrokerAdapterError::Rejected)?;
    if status_line.starts_with(b"HTTP/1.1 429") || status_line.starts_with(b"HTTP/1.1 503") {
        return Err(BrokerAdapterError::Retryable);
    }
    if status_line.starts_with(b"HTTP/1.1 401") || status_line.starts_with(b"HTTP/1.1 403") {
        return Ok(BrokerResponse {
            status: BrokerStatus::Rejected,
            body: Vec::new(),
        });
    }
    if !status_line.starts_with(b"HTTP/1.1 200")
        || body
            .windows(credential.len())
            .any(|window| window == credential)
    {
        return Err(BrokerAdapterError::Rejected);
    }
    let reply: CompletionReply =
        serde_json::from_slice(body).map_err(|_| BrokerAdapterError::Rejected)?;
    if reply.result.len() > 4_096 || reply.result.contains(['\r', '\n', '\0']) {
        return Err(BrokerAdapterError::Rejected);
    }
    let body = serde_json::to_vec(&reply).map_err(|_| BrokerAdapterError::Rejected)?;
    Ok(BrokerResponse {
        status: BrokerStatus::Succeeded,
        body,
    })
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CompletionReply {
    result: String,
}

fn valid_dns_destination(value: &str) -> bool {
    let labels = value.split('.').collect::<Vec<_>>();
    (1..=253).contains(&value.len())
        && labels.len() >= 2
        && labels.iter().all(|label| {
            (1..=63).contains(&label.len())
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
                && label
                    .as_bytes()
                    .first()
                    .is_some_and(u8::is_ascii_alphanumeric)
                && label
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_alphanumeric)
        })
        && value.parse::<std::net::IpAddr>().is_err()
        && !matches!(
            labels.last().copied(),
            Some("internal" | "local" | "localhost")
        )
}

const fn valid_bearer_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~' | b'+' | b'/' | b'=')
}

/// Bound broker listener ready to accept provider-forwarded vsock streams.
pub struct BrokerServer {
    socket_path: PathBuf,
    listener: UnixListener,
    executor: Arc<dyn BrokerExecutor>,
}

impl Drop for BrokerServer {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.socket_path)
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            drop(fs::remove_file(&self.socket_path));
        }
    }
}

impl BrokerServer {
    /// Binds a private host socket.
    ///
    /// # Errors
    ///
    /// Rejects non-absolute paths, unsafe existing objects, filesystem
    /// failures, and sockets whose permissions cannot be restricted.
    pub fn bind(
        socket_path: impl Into<PathBuf>,
        executor: Arc<dyn BrokerExecutor>,
    ) -> Result<Self, BrokerServerError> {
        let socket_path = socket_path.into();
        if !socket_path.is_absolute() {
            return Err(BrokerServerError::UnsafeSocket);
        }
        if let Ok(metadata) = fs::symlink_metadata(&socket_path) {
            if !metadata.file_type().is_socket() || metadata.file_type().is_symlink() {
                return Err(BrokerServerError::UnsafeSocket);
            }
            fs::remove_file(&socket_path)?;
        }
        let listener = UnixListener::bind(&socket_path)?;
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;
        Ok(Self {
            socket_path,
            listener,
            executor,
        })
    }

    /// Returns the exact provider-facing socket path.
    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// Serves connections until cancellation.
    ///
    /// # Errors
    ///
    /// Returns listener failures. Individual malformed connections are denied
    /// and closed without stopping the listener.
    pub async fn serve(self, cancellation: CancellationToken) -> Result<(), BrokerServerError> {
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                accepted = self.listener.accept() => {
                    let (stream, _) = accepted?;
                    let executor = Arc::clone(&self.executor);
                    tokio::spawn(async move {
                        if let Err(error) = serve_connection(stream, executor).await {
                            tracing::warn!(error_class = error.code(), "broker connection closed");
                        }
                    });
                }
            }
        }
    }
}

async fn serve_connection(
    mut stream: UnixStream,
    executor: Arc<dyn BrokerExecutor>,
) -> Result<(), BrokerServerError> {
    loop {
        let length = match stream.read_u32().await {
            Ok(length) => usize::try_from(length).map_err(|_| BrokerServerError::OversizedFrame)?,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if length == 0 || length > MAX_FRAME_BYTES {
            return Err(BrokerServerError::OversizedFrame);
        }
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload).await?;
        let request = serde_json::from_slice(&payload).map_err(|_| BrokerServerError::Malformed)?;
        let response = executor.execute(request).await;
        let encoded = serde_json::to_vec(&response).map_err(|_| BrokerServerError::Malformed)?;
        write_frame(&mut stream, &encoded).await?;
    }
}

async fn write_frame(stream: &mut UnixStream, payload: &[u8]) -> Result<(), BrokerServerError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(BrokerServerError::OversizedFrame);
    }
    let length = u32::try_from(payload.len()).map_err(|_| BrokerServerError::OversizedFrame)?;
    stream.write_u32(length).await?;
    stream.write_all(payload).await?;
    stream.flush().await?;
    Ok(())
}

const fn denied() -> WireBrokerResponse {
    WireBrokerResponse {
        status: WireBrokerStatus::Denied,
        body: Vec::new(),
    }
}

const fn error_class(error: &secret_application::SecretServiceError) -> &'static str {
    match error {
        secret_application::SecretServiceError::BrokerAdapter(BrokerAdapterError::Retryable) => {
            "adapter_retryable"
        }
        secret_application::SecretServiceError::BrokerAdapter(BrokerAdapterError::Rejected) => {
            "adapter_rejected"
        }
        _ => "denied",
    }
}

/// Broker listener or framing failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BrokerServerError {
    /// Socket path or pre-existing object is unsafe.
    #[error("broker socket path is unsafe")]
    UnsafeSocket,
    /// Filesystem or stream operation failed.
    #[error("broker transport I/O failed")]
    Io(#[from] std::io::Error),
    /// Frame exceeded the protocol bound.
    #[error("broker frame exceeds the protocol bound")]
    OversizedFrame,
    /// Frame was not valid canonical protocol JSON.
    #[error("broker frame is malformed")]
    Malformed,
}

impl BrokerServerError {
    const fn code(&self) -> &'static str {
        match self {
            Self::UnsafeSocket => "unsafe_socket",
            Self::Io(_) => "io",
            Self::OversizedFrame => "oversized_frame",
            Self::Malformed => "malformed",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use brokered_egress_domain::{
        BrokeredSecretRuleId, ExactHttpsOrigin, HeaderName, HttpInjectionLocation,
    };
    use std::sync::Mutex;
    use tempfile::TempDir;
    use tokio::net::TcpListener;
    use uuid::Uuid;

    fn outbound_rule() -> BrokeredSecretRule {
        BrokeredSecretRule {
            id: BrokeredSecretRuleId::new(),
            binding_id: Uuid::new_v4(),
            instance_revision_id: Uuid::new_v4(),
            secret_version_id: Uuid::new_v4(),
            destination: Some(ExactHttpsOrigin::parse("https://api.example.test").expect("origin")),
            location: HttpInjectionLocation::OutboundHeaderPrefix {
                header: HeaderName::parse("authorization").expect("header"),
                prefix: String::from("Bearer "),
            },
            gateway_route_id: None,
        }
    }

    struct RecordingExecutor {
        seen: Mutex<Vec<u8>>,
    }

    #[derive(Default)]
    struct RecordingHttpsTransport {
        seen: Mutex<Vec<BrokeredHttpsHeader>>,
        response: Vec<u8>,
    }

    #[async_trait]
    impl PinnedHttpsTransport for RecordingHttpsTransport {
        async fn send(
            &self,
            request: UpstreamHttpsRequest,
        ) -> Result<BrokerResponse, BrokerAdapterError> {
            assert_eq!(request.destination(), "api.example.test");
            assert_eq!(request.path_and_query(), "/v1/messages?bounded=true");
            *self.seen.lock().expect("recording lock") = request.headers;
            Ok(BrokerResponse {
                status: BrokerStatus::Succeeded,
                body: if self.response.is_empty() {
                    b"ordinary response".to_vec()
                } else {
                    self.response.clone()
                },
            })
        }
    }

    #[async_trait]
    impl BrokerExecutor for RecordingExecutor {
        async fn execute(&self, request: WireBrokerRequest) -> WireBrokerResponse {
            *self.seen.lock().expect("recording lock") = request.credential;
            WireBrokerResponse {
                status: WireBrokerStatus::Succeeded,
                body: b"sanitized".to_vec(),
            }
        }
    }

    #[tokio::test]
    async fn framed_transport_never_echoes_runtime_credential() {
        let temporary = TempDir::new().expect("temporary directory");
        let socket = temporary.path().join("broker.sock");
        let executor = Arc::new(RecordingExecutor {
            seen: Mutex::new(Vec::new()),
        });
        let server = BrokerServer::bind(socket.clone(), executor.clone()).expect("bind broker");
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(server.serve(cancellation.clone()));
        let mut client = UnixStream::connect(&socket).await.expect("connect broker");
        let sentinel = b"runtime-credential-sentinel".to_vec();
        let request = WireBrokerRequest {
            credential: sentinel.clone(),
            run_id: RunId::new(),
            slot: String::from("model"),
            destination: String::from("api.example.test"),
            operation: String::from("complete"),
            body: b"request".to_vec(),
        };
        let encoded = serde_json::to_vec(&request).expect("encode request");
        write_frame(&mut client, &encoded)
            .await
            .expect("write request");
        let response_length = client.read_u32().await.expect("response length");
        let mut response = vec![0_u8; response_length as usize];
        client
            .read_exact(&mut response)
            .await
            .expect("response payload");
        assert!(
            !response
                .windows(sentinel.len())
                .any(|value| value == sentinel)
        );
        let response: WireBrokerResponse =
            serde_json::from_slice(&response).expect("decode response");
        assert_eq!(response.status, WireBrokerStatus::Succeeded);
        assert_eq!(
            executor.seen.lock().expect("recording lock").as_slice(),
            sentinel
        );
        cancellation.cancel();
        task.await.expect("broker task").expect("broker shutdown");
    }

    #[tokio::test]
    async fn loopback_adapter_applies_bearer_and_returns_only_sanitized_json() {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake upstream");
        let address = listener.local_addr().expect("fake upstream address");
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept fake request");
            let mut request = Vec::new();
            stream
                .read_to_end(&mut request)
                .await
                .expect("read fake request");
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\
                      Connection: close\r\n\r\n{\"result\":\"accepted\"}",
                )
                .await
                .expect("write fake response");
            request
        });
        let adapter =
            LoopbackCompletionAdapter::new("api.example.test", address, 1).expect("adapter");
        let sentinel = b"brokered-credential-sentinel";
        let response = adapter
            .invoke(
                &SecretValue::new(sentinel).expect("credential"),
                "api.example.test",
                "complete",
                br#"{"prompt":"bounded"}"#,
            )
            .await
            .expect("authorized fake operation");
        assert_eq!(response.status, BrokerStatus::Succeeded);
        assert_eq!(response.body, br#"{"result":"accepted"}"#);
        assert!(
            !response
                .body
                .windows(sentinel.len())
                .any(|window| window == sentinel)
        );
        let request = upstream.await.expect("fake upstream task");
        let authorization = [b"Authorization: Bearer ".as_slice(), sentinel.as_slice()].concat();
        assert!(
            request
                .windows(authorization.len())
                .any(|window| window == authorization)
        );
        assert!(
            request
                .windows(b"Host: api.example.test".len())
                .any(|window| window == b"Host: api.example.test")
        );
    }

    #[tokio::test]
    async fn loopback_adapter_rejects_redirects_credential_echo_and_address_injection() {
        assert!(matches!(
            LoopbackCompletionAdapter::new("127.0.0.1", "127.0.0.1:1".parse().expect("address"), 1),
            Err(BrokerAdapterError::Rejected)
        ));
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("fake upstream");
        let address = listener.local_addr().expect("fake upstream address");
        let upstream = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept fake request");
            let mut request = Vec::new();
            stream
                .read_to_end(&mut request)
                .await
                .expect("read fake request");
            stream
                .write_all(
                    b"HTTP/1.1 302 Found\r\nLocation: http://169.254.169.254/\r\n\
                      Connection: close\r\n\r\n{\"result\":\"brokered-credential-sentinel\"}",
                )
                .await
                .expect("write fake response");
        });
        let adapter =
            LoopbackCompletionAdapter::new("api.example.test", address, 1).expect("adapter");
        let result = adapter
            .invoke(
                &SecretValue::new("brokered-credential-sentinel").expect("credential"),
                "api.example.test",
                "complete",
                b"{}",
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
        upstream.await.expect("fake upstream task");
    }

    #[tokio::test]
    async fn loopback_adapter_rate_limits_before_connecting_upstream() {
        let adapter = LoopbackCompletionAdapter::new(
            "api.example.test",
            "127.0.0.1:1".parse().expect("address"),
            1,
        )
        .expect("adapter");
        let _occupied = Arc::clone(&adapter.concurrency)
            .acquire_owned()
            .await
            .expect("occupy adapter");
        let result = adapter
            .invoke(
                &SecretValue::new("bounded-credential").expect("credential"),
                "api.example.test",
                "complete",
                b"{}",
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Retryable)));
    }

    #[tokio::test]
    async fn generic_https_adapter_substitutes_only_the_bound_placeholder() {
        let transport = RecordingHttpsTransport::default();
        let rule = outbound_rule();
        let placeholder = rule.placeholder();
        let rule_id = rule.id.as_uuid();
        let adapter = BrokeredHttpsAdapter::new(rule, transport).expect("valid adapter");
        let request = BrokeredHttpsRequest {
            rule_id,
            method: BrokeredHttpsMethod::Post,
            path_and_query: String::from("/v1/messages?bounded=true"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: format!("Bearer {placeholder}"),
            }],
            body: br#"{\"message\":\"hello\"}"#.to_vec(),
        };
        let response = adapter
            .invoke(
                &SecretValue::new("real-secret-sentinel").expect("secret"),
                "api.example.test",
                "https_v1",
                &serde_json::to_vec(&request).expect("request"),
            )
            .await
            .expect("authorized request");
        assert_eq!(response.body, b"ordinary response");
        let headers = adapter.transport.seen.lock().expect("recording lock");
        assert_eq!(headers.len(), 1);
        assert_eq!(headers[0].value, "Bearer real-secret-sentinel");
        drop(headers);
        assert!(
            !response
                .body
                .windows(b"real-secret-sentinel".len())
                .any(|window| window == b"real-secret-sentinel")
        );
    }

    #[tokio::test]
    async fn generic_https_adapter_rejects_an_upstream_secret_echo() {
        let rule = outbound_rule();
        let placeholder = rule.placeholder();
        let rule_id = rule.id.as_uuid();
        let adapter = BrokeredHttpsAdapter::new(
            rule,
            RecordingHttpsTransport {
                seen: Mutex::new(Vec::new()),
                response: b"upstream echoed real-secret-sentinel".to_vec(),
            },
        )
        .expect("valid adapter");
        let request = BrokeredHttpsRequest {
            rule_id,
            method: BrokeredHttpsMethod::Post,
            path_and_query: String::from("/v1/messages?bounded=true"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: format!("Bearer {placeholder}"),
            }],
            body: Vec::new(),
        };
        let result = adapter
            .invoke(
                &SecretValue::new("real-secret-sentinel").expect("secret"),
                "api.example.test",
                "https_v1",
                &serde_json::to_vec(&request).expect("request"),
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    }

    #[tokio::test]
    async fn generic_https_adapter_rejects_ambiguous_or_unbound_requests() {
        let rule = outbound_rule();
        let placeholder = rule.placeholder();
        let adapter = BrokeredHttpsAdapter::new(rule, RecordingHttpsTransport::default())
            .expect("valid adapter");
        let credential = SecretValue::new("real-secret").expect("secret");
        for request in [
            BrokeredHttpsRequest {
                rule_id: uuid::Uuid::new_v4(),
                method: BrokeredHttpsMethod::Get,
                path_and_query: String::from("https://metadata.google.internal/"),
                headers: vec![BrokeredHttpsHeader {
                    name: String::from("authorization"),
                    value: format!("Bearer {placeholder}"),
                }],
                body: Vec::new(),
            },
            BrokeredHttpsRequest {
                rule_id: uuid::Uuid::new_v4(),
                method: BrokeredHttpsMethod::Get,
                path_and_query: String::from("/safe"),
                headers: vec![
                    BrokeredHttpsHeader {
                        name: String::from("authorization"),
                        value: format!("Bearer {placeholder}"),
                    },
                    BrokeredHttpsHeader {
                        name: String::from("authorization"),
                        value: format!("Bearer {placeholder}"),
                    },
                ],
                body: Vec::new(),
            },
            BrokeredHttpsRequest {
                rule_id: uuid::Uuid::new_v4(),
                method: BrokeredHttpsMethod::Get,
                path_and_query: String::from("/safe"),
                headers: vec![BrokeredHttpsHeader {
                    name: String::from("authorization"),
                    value: String::from("Bearer different-placeholder"),
                }],
                body: Vec::new(),
            },
        ] {
            let result = adapter
                .invoke(
                    &credential,
                    "api.example.test",
                    "https_v1",
                    &serde_json::to_vec(&request).expect("request"),
                )
                .await;
            assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
        }
        let request = BrokeredHttpsRequest {
            rule_id: uuid::Uuid::new_v4(),
            method: BrokeredHttpsMethod::Get,
            path_and_query: String::from("/safe"),
            headers: vec![BrokeredHttpsHeader {
                name: String::from("authorization"),
                value: format!("Bearer {placeholder}"),
            }],
            body: Vec::new(),
        };
        let result = adapter
            .invoke(
                &credential,
                "other.example.test",
                "https_v1",
                &serde_json::to_vec(&request).expect("request"),
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    }

    #[test]
    fn pinned_transport_rejects_private_and_unpinned_addresses() {
        for addresses in [
            Vec::new(),
            vec!["127.0.0.1".parse().expect("address")],
            vec!["169.254.169.254".parse().expect("address")],
            vec!["::1".parse().expect("address")],
            vec!["fc00::1".parse().expect("address")],
        ] {
            assert!(matches!(
                ReqwestPinnedHttpsTransport::new("api.example.test", &addresses),
                Err(BrokerAdapterError::Rejected)
            ));
        }
    }

    #[tokio::test]
    async fn loopback_adapter_rejects_dns_and_body_confusion_before_connecting() {
        for destination in [
            "::1",
            "[::1]",
            "169.254.169.254",
            "metadata.google.internal",
            "localhost.local",
            "-api.example.test",
            "api..example.test",
        ] {
            assert!(matches!(
                LoopbackCompletionAdapter::new(
                    destination,
                    "127.0.0.1:1".parse().expect("address"),
                    1
                ),
                Err(BrokerAdapterError::Rejected)
            ));
        }
        let adapter = LoopbackCompletionAdapter::new(
            "api.example.test",
            "127.0.0.1:1".parse().expect("address"),
            1,
        )
        .expect("adapter");
        for destination in ["alternate.example.test", "api.example.test."] {
            let result = adapter
                .invoke(
                    &SecretValue::new("bounded-credential").expect("credential"),
                    destination,
                    "complete",
                    b"{}",
                )
                .await;
            assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
        }
        let oversized = vec![b'x'; MAX_ADAPTER_BODY_BYTES + 1];
        let result = adapter
            .invoke(
                &SecretValue::new("bounded-credential").expect("credential"),
                "api.example.test",
                "complete",
                &oversized,
            )
            .await;
        assert!(matches!(result, Err(BrokerAdapterError::Rejected)));
    }

    #[test]
    fn sanitizer_rejects_oversized_and_malicious_upstream_responses() {
        let oversized = vec![
            b'x';
            usize::try_from(MAX_UPSTREAM_RESPONSE_BYTES)
                .expect("response bound fits usize")
                + 1
        ];
        assert!(matches!(
            sanitize_upstream_response(&oversized, b"credential"),
            Err(BrokerAdapterError::Rejected)
        ));
        for response in [
            b"HTTP/1.1 200 OK\r\nX-Injected: yes\r\n\r\n{\"result\":\"line\\nfeed\"}".as_slice(),
            b"HTTP/1.0 200 OK\r\n\r\n{\"result\":\"accepted\"}".as_slice(),
            b"HTTP/1.1 200 OK\r\n\r\n{\"result\":\"accepted\",\"token\":\"leak\"}".as_slice(),
        ] {
            assert!(matches!(
                sanitize_upstream_response(response, b"credential"),
                Err(BrokerAdapterError::Rejected)
            ));
        }
    }
}
