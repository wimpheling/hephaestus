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
#[path = "service_probe/tests.rs"]
mod tests;
