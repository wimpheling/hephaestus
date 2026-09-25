use std::time::Duration;

use crate::{GatewayEdgeError, GatewayLimits};

pub(super) const DEFAULT_WIRE_HEADER_BYTES: usize = 64 * 1024;
const MAX_POLICY_BODY_BYTES: usize = 16 * 1024 * 1024;
const MAX_POLICY_HEADERS: usize = 4_096;
const MAX_POLICY_PATH_BYTES: usize = 64 * 1024;
const MAX_POLICY_WIRE_HEADER_BYTES: usize = 4 * 1024 * 1024;
const MAX_POLICY_TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Bounds for one normalized private service HTTP exchange.
#[derive(Debug, Clone, Copy)]
pub struct ServiceHttpPolicy {
    /// Maximum request body bytes.
    pub max_request_body_bytes: usize,
    /// Maximum response body bytes.
    pub max_response_body_bytes: usize,
    /// Maximum request header values.
    pub max_request_headers: usize,
    /// Maximum response header values.
    pub max_response_headers: usize,
    /// Maximum canonical path and query bytes.
    pub max_path_and_query_bytes: usize,
    /// Maximum canonical serialized header bytes in either direction.
    ///
    /// Hyper's HTTP/1 parser buffer is clamped to a minimum of 8 KiB, so this
    /// bound is an aggregate canonical-header limit rather than a raw-wire
    /// limit below that parser minimum.
    pub max_wire_header_bytes: usize,
    /// Total request/response exchange deadline.
    pub exchange_timeout: Duration,
}

impl ServiceHttpPolicy {
    /// Builds service HTTP bounds from the route's canonical gateway limits.
    #[must_use]
    pub const fn from_gateway_limits(limits: GatewayLimits) -> Self {
        Self {
            max_request_body_bytes: limits.max_request_body_bytes,
            max_response_body_bytes: limits.max_response_body_bytes,
            max_request_headers: limits.max_request_headers,
            max_response_headers: limits.max_response_headers,
            max_path_and_query_bytes: limits.max_path_and_query_bytes,
            max_wire_header_bytes: DEFAULT_WIRE_HEADER_BYTES,
            exchange_timeout: limits.execution_timeout,
        }
    }

    /// Overrides the explicit serialized-header bound for focused deployments.
    #[must_use]
    pub const fn with_max_wire_header_bytes(mut self, maximum: usize) -> Self {
        self.max_wire_header_bytes = maximum;
        self
    }

    pub(crate) fn validate(self) -> Result<(), GatewayEdgeError> {
        if self.max_request_body_bytes == 0
            || self.max_request_body_bytes > MAX_POLICY_BODY_BYTES
            || self.max_response_body_bytes == 0
            || self.max_response_body_bytes > MAX_POLICY_BODY_BYTES
            || self.max_request_headers == 0
            || self.max_request_headers > MAX_POLICY_HEADERS
            || self.max_response_headers == 0
            || self.max_response_headers > MAX_POLICY_HEADERS
            || self.max_path_and_query_bytes == 0
            || self.max_path_and_query_bytes > MAX_POLICY_PATH_BYTES
            || self.max_wire_header_bytes == 0
            || self.max_wire_header_bytes > MAX_POLICY_WIRE_HEADER_BYTES
            || self.exchange_timeout.is_zero()
            || self.exchange_timeout > MAX_POLICY_TIMEOUT
        {
            return Err(GatewayEdgeError::Contract(
                "invalid private service HTTP policy",
            ));
        }
        Ok(())
    }
}
