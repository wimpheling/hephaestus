use super::{
    common::{
        Arc, AsyncReadExt, AsyncWriteExt, BrokerAdapter, BrokerAdapterError, BrokerRequest,
        BrokerResponse, BrokerStatus, MAX_ADAPTER_BODY_BYTES, MAX_CREDENTIAL_BYTES,
        MAX_UPSTREAM_RESPONSE_BYTES, OpaqueRuntimeCredential, SecretRuntimeResolver, SecretSlotKey,
        SecretValue, SocketAddr, TcpStream, UPSTREAM_TIMEOUT, WireBrokerRequest,
        WireBrokerResponse, WireBrokerStatus,
    },
    validation::{valid_bearer_byte, valid_dns_destination},
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tokio::sync::Semaphore;

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
    pub(super) concurrency: Arc<Semaphore>,
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

pub(super) const fn denied() -> WireBrokerResponse {
    WireBrokerResponse {
        status: WireBrokerStatus::Denied,
        body: Vec::new(),
    }
}

pub(super) const fn error_class(error: &secret_application::SecretServiceError) -> &'static str {
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

pub(super) fn sanitize_upstream_response(
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
