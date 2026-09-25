//! Bounded HTTP readiness and health probes for one private service VM.

use bytes::Bytes;
use gateway_domain::ServiceProbePath;
use http::{HeaderMap, HeaderValue, Method, StatusCode};
use std::{
    net::{IpAddr, Ipv4Addr},
    time::Duration,
};
use tokio::time;
use uuid::Uuid;
use vm_trait::{VmError, VmInstance};

use crate::{
    GatewayEdgeError, GatewayRequest, GatewayScheme, ServiceHttpPolicy, TrustedRequestMetadata,
    exchange_private_service_http,
};

const DEFAULT_PROBE_BODY_BYTES: usize = 8 * 1024;
const DEFAULT_PROBE_HEADERS: usize = 32;
const DEFAULT_PROBE_WIRE_HEADER_BYTES: usize = 8 * 1024;
const MAX_PROBE_TIMEOUT: Duration = Duration::from_secs(30);

/// Platform-owned bounds for one readiness or health probe.
#[derive(Debug, Clone, Copy)]
pub struct ServiceProbePolicy {
    timeout: Duration,
}

impl ServiceProbePolicy {
    /// Creates a probe policy with bounded platform defaults.
    #[must_use]
    pub const fn new(timeout: Duration) -> Self {
        Self { timeout }
    }

    fn validate(self) -> Result<(), ServiceProbeError> {
        if self.timeout.is_zero() || self.timeout > MAX_PROBE_TIMEOUT {
            return Err(ServiceProbeError::InvalidPolicy);
        }
        Ok(())
    }
}

/// Successful bounded probe result. Probe bodies are intentionally discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceProbeSuccess {
    /// The exact successful HTTP status returned by the service.
    pub status: StatusCode,
}

/// Redacted readiness or health probe failure.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ServiceProbeError {
    /// The caller supplied a zero or unreasonably large timeout.
    #[error("invalid service probe policy")]
    InvalidPolicy,
    /// The trusted authority cannot be represented as an HTTP host value.
    #[error("invalid service probe authority")]
    InvalidAuthority,
    /// The one total probe deadline elapsed during connection or exchange.
    #[error("service probe timed out")]
    TimedOut,
    /// The private stream or VM was unavailable.
    #[error("service probe unavailable")]
    Unavailable,
    /// The private response violated the bounded HTTP contract.
    #[error("service probe response violated the HTTP contract")]
    Contract,
    /// The service responded, but not with a successful 2xx status.
    #[error("service probe returned a non-success status")]
    NonSuccess(StatusCode),
}

/// Performs one synthetic GET probe over a running VM's private service stream.
///
/// The deadline starts before opening the private connection and the remaining
/// duration is passed to the bounded HTTP exchange. The request has no caller
/// headers, secret substitution, invocation identity, or reusable bearer.
///
/// # Errors
///
/// Returns a redacted error when the policy, authority, private transport,
/// deadline, HTTP framing, or service status violates the probe contract.
pub async fn probe_private_service_http(
    vm: &dyn VmInstance,
    path: &ServiceProbePath,
    trusted_authority: &str,
    policy: ServiceProbePolicy,
) -> Result<ServiceProbeSuccess, ServiceProbeError> {
    policy.validate()?;
    let authority = HeaderValue::try_from(trusted_authority)
        .map_err(|_| ServiceProbeError::InvalidAuthority)?;
    if authority.is_empty() {
        return Err(ServiceProbeError::InvalidAuthority);
    }
    let deadline = time::Instant::now()
        .checked_add(policy.timeout)
        .ok_or(ServiceProbeError::InvalidPolicy)?;
    let connection = time::timeout_at(deadline, vm.open_private_service_connection())
        .await
        .map_err(|_| ServiceProbeError::TimedOut)?
        .map_err(map_vm_error)?;
    let remaining = deadline.saturating_duration_since(time::Instant::now());
    if remaining.is_zero() {
        return Err(ServiceProbeError::TimedOut);
    }
    let request = GatewayRequest {
        method: Method::GET,
        path_and_query: path.as_str().to_owned(),
        headers: HeaderMap::new(),
        body: Bytes::new(),
        trusted: TrustedRequestMetadata {
            scheme: GatewayScheme::Http,
            authority: authority
                .to_str()
                .map_err(|_| ServiceProbeError::InvalidAuthority)?
                .to_owned(),
            client_address: IpAddr::V4(Ipv4Addr::LOCALHOST),
            request_id: Uuid::new_v4(),
        },
    };
    let exchange_policy = ServiceHttpPolicy {
        max_request_body_bytes: 1,
        max_response_body_bytes: DEFAULT_PROBE_BODY_BYTES,
        max_request_headers: 1,
        max_response_headers: DEFAULT_PROBE_HEADERS,
        max_path_and_query_bytes: 512,
        max_wire_header_bytes: DEFAULT_PROBE_WIRE_HEADER_BYTES,
        exchange_timeout: remaining,
    };
    match time::timeout_at(
        deadline,
        exchange_private_service_http(connection, request, exchange_policy),
    )
    .await
    {
        Err(_) => Err(ServiceProbeError::TimedOut),
        Ok(Ok(response)) if response.status.is_success() => Ok(ServiceProbeSuccess {
            status: response.status,
        }),
        Ok(Ok(response)) => Err(ServiceProbeError::NonSuccess(response.status)),
        Ok(Err(error)) => Err(map_exchange_error(&error)),
    }
}

fn map_vm_error(_: VmError) -> ServiceProbeError {
    ServiceProbeError::Unavailable
}

const fn map_exchange_error(error: &GatewayEdgeError) -> ServiceProbeError {
    match error {
        GatewayEdgeError::Contract(_) => ServiceProbeError::Contract,
        GatewayEdgeError::HandlerUnavailable => ServiceProbeError::TimedOut,
        _ => ServiceProbeError::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::{
        sync::{Arc, Mutex},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt, DuplexStream},
        sync::{broadcast, oneshot},
    };
    use vm_trait::{BoxedPrivateServiceConnection, StopMode, VmEvent, VmExit, VmId};

    struct ProbeVm {
        id: VmId,
        connection: Mutex<Option<BoxedPrivateServiceConnection>>,
        open_delay: Duration,
        events: broadcast::Sender<VmEvent>,
    }

    impl ProbeVm {
        fn new(connection: BoxedPrivateServiceConnection, open_delay: Duration) -> Arc<Self> {
            let (events, _) = broadcast::channel(4);
            Arc::new(Self {
                id: VmId(String::from("probe-vm")),
                connection: Mutex::new(Some(connection)),
                open_delay,
                events,
            })
        }
    }

    #[async_trait]
    impl VmInstance for ProbeVm {
        fn id(&self) -> &VmId {
            &self.id
        }

        async fn start(&self) -> Result<(), VmError> {
            Ok(())
        }

        async fn stop(&self, _: StopMode) -> Result<(), VmError> {
            Ok(())
        }

        async fn wait(&self) -> Result<VmExit, VmError> {
            Ok(VmExit {
                code: Some(0),
                signal: None,
            })
        }

        async fn open_private_service_connection(
            &self,
        ) -> Result<BoxedPrivateServiceConnection, VmError> {
            time::sleep(self.open_delay).await;
            self.connection
                .lock()
                .expect("probe connection lock")
                .take()
                .ok_or_else(|| VmError::Unavailable {
                    resource: String::from("probe connection"),
                    reason: String::from("connection already consumed"),
                })
        }

        fn subscribe_events(&self) -> broadcast::Receiver<VmEvent> {
            self.events.subscribe()
        }

        async fn destroy(&self) -> Result<(), VmError> {
            Ok(())
        }
    }

    fn path(value: &str) -> ServiceProbePath {
        ServiceProbePath::parse(value).expect("probe path")
    }

    fn pair(open_delay: Duration) -> (Arc<ProbeVm>, DuplexStream) {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let vm = ProbeVm::new(Box::new(client), open_delay);
        (vm, server)
    }

    async fn read_request_headers(stream: &mut DuplexStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        loop {
            let count = stream.read(&mut buffer).await.expect("read probe request");
            assert_ne!(count, 0, "probe closed before request headers");
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                return request;
            }
        }
    }

    #[tokio::test]
    async fn uses_declared_path_and_trusted_host_without_caller_headers() {
        let (vm, mut server) = pair(Duration::ZERO);
        let server_task = tokio::spawn(async move {
            let request = read_request_headers(&mut server).await;
            let request = String::from_utf8(request).expect("HTTP request text");
            assert!(request.starts_with("GET /ready HTTP/1.1\r\n"));
            assert!(request.contains("host: service.internal\r\n"));
            assert!(request.contains("content-length: 0\r\n"));
            assert!(request.contains("connection: close\r\n"));
            assert!(!request.contains("authorization"));
            server
                .write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                .await
                .expect("write probe response");
        });
        vm.start().await.expect("control bootstrap");
        let result = probe_private_service_http(
            vm.as_ref(),
            &path("/ready"),
            "service.internal",
            ServiceProbePolicy::new(Duration::from_secs(1)),
        )
        .await
        .expect("successful readiness probe");
        assert_eq!(result.status, StatusCode::NO_CONTENT);
        server_task.await.expect("probe server task");
    }

    #[tokio::test]
    async fn rejects_non_success_and_oversized_probe_responses() {
        for response in [
            &b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nContent-Length: 8193\r\n\r\n"[..],
        ] {
            let (vm, mut server) = pair(Duration::ZERO);
            let server_task = tokio::spawn(async move {
                read_request_headers(&mut server).await;
                server
                    .write_all(response)
                    .await
                    .expect("write probe response");
            });
            let result = probe_private_service_http(
                vm.as_ref(),
                &path("/health"),
                "service.internal",
                ServiceProbePolicy::new(Duration::from_secs(1)),
            )
            .await;
            match response[9] {
                b'5' => assert_eq!(
                    result,
                    Err(ServiceProbeError::NonSuccess(
                        StatusCode::SERVICE_UNAVAILABLE
                    ))
                ),
                b'2' => assert_eq!(result, Err(ServiceProbeError::Contract)),
                status => panic!("unexpected fixture status {status}"),
            }
            server_task.await.expect("probe server task");
        }
    }

    #[tokio::test]
    async fn deadline_covers_open_and_exchange_after_control_start() {
        let (vm, mut server) = pair(Duration::from_millis(15));
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            time::sleep(Duration::from_millis(25)).await;
            let _ = server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await;
        });
        vm.start().await.expect("control bootstrap alone");
        let result = probe_private_service_http(
            vm.as_ref(),
            &path("/ready"),
            "service.internal",
            ServiceProbePolicy::new(Duration::from_millis(30)),
        )
        .await;
        assert_eq!(result, Err(ServiceProbeError::TimedOut));
        server_task.await.expect("probe server task");
    }

    #[tokio::test]
    async fn caller_cancellation_closes_probe_stream() {
        let (vm, mut server) = pair(Duration::ZERO);
        let (seen_tx, seen_rx) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            seen_tx.send(()).expect("signal probe request");
            let mut byte = [0_u8; 1];
            let count = server.read(&mut byte).await.expect("observe probe close");
            assert_eq!(count, 0, "cancelled probe kept stream alive");
        });
        let probe = tokio::spawn({
            let vm = Arc::clone(&vm);
            async move {
                probe_private_service_http(
                    vm.as_ref(),
                    &path("/ready"),
                    "service.internal",
                    ServiceProbePolicy::new(Duration::from_secs(1)),
                )
                .await
            }
        });
        seen_rx.await.expect("probe request reached server");
        probe.abort();
        assert!(probe.await.expect_err("probe was cancelled").is_cancelled());
        server_task.await.expect("probe server task");
    }
}
