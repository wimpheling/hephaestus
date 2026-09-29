use http::{HeaderValue, StatusCode, header, header::CONTENT_LENGTH};
use http_body_util::{BodyExt, Limited};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use tokio::{task::JoinHandle, time};
use vm_trait::BoxedPrivateServiceConnection;

use super::{
    policy::ServiceHttpPolicy,
    request::canonical_request,
    response::{response_framing, response_headers_without_transport, validate_response_headers},
};
use crate::{GatewayEdgeError, GatewayRequest, GatewayResponse};

const MIN_HTTP1_BUFFER_BYTES: usize = 8 * 1024;

/// Performs one bounded request/response exchange over an authenticated VM
/// service stream.
///
/// The stream is deliberately used once. The request is canonicalized before
/// it reaches the guest, and the Hyper connection driver is owned by a guard
/// that aborts it when this future is cancelled.
///
/// # Errors
///
/// Returns a contract error when framing, headers, paths, or byte bounds are
/// invalid, and a handler-unavailable error when the private stream or its
/// deadline fails.
pub async fn exchange(
    connection: BoxedPrivateServiceConnection,
    request: GatewayRequest,
    policy: ServiceHttpPolicy,
) -> Result<GatewayResponse, GatewayEdgeError> {
    policy.validate()?;
    let request_method = request.method.clone();
    let request = canonical_request(request, policy)?;
    let deadline = time::Instant::now() + policy.exchange_timeout;
    let mut builder = http1::Builder::new();
    builder
        .max_headers(policy.max_response_headers)
        .max_buf_size(policy.max_wire_header_bytes.max(MIN_HTTP1_BUFFER_BYTES));
    let (mut sender, driver) =
        time::timeout_at(deadline, builder.handshake(TokioIo::new(connection)))
            .await
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
            .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
    let _driver = DriverGuard(tokio::spawn(driver));

    let response = time::timeout_at(deadline, sender.send_request(request))
        .await
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;
    validate_response_headers(&response, policy)?;
    let framing = response_framing(&response, &request_method)?;
    let status = response.status();
    if status == StatusCode::SWITCHING_PROTOCOLS || response.headers().contains_key(header::UPGRADE)
    {
        return Err(GatewayEdgeError::Contract(
            "service HTTP upgrades are unsupported",
        ));
    }
    if response.headers().contains_key(header::TRAILER) {
        return Err(GatewayEdgeError::Contract(
            "service response trailers are unsupported",
        ));
    }
    let max_body = policy.max_response_body_bytes;
    let content_length = framing.content_length();
    if !framing.is_bodyless() && content_length.is_some_and(|length| length > max_body) {
        return Err(GatewayEdgeError::Contract(
            "service response exceeds the body limit",
        ));
    }
    let response_headers = response.headers().clone();
    let collected = time::timeout_at(
        deadline,
        Limited::new(response.into_body(), max_body).collect(),
    )
    .await
    .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
    .map_err(|_| GatewayEdgeError::Contract("service response exceeds the body limit"))?;
    if collected.trailers().is_some() {
        return Err(GatewayEdgeError::Contract(
            "service response trailers are unsupported",
        ));
    }
    let body = collected.to_bytes();
    if !framing.is_bodyless() && content_length.is_some_and(|length| length != body.len()) {
        return Err(GatewayEdgeError::Contract(
            "service response length is inconsistent",
        ));
    }
    if framing.is_bodyless() && !body.is_empty() {
        return Err(GatewayEdgeError::Contract(
            "body received for a bodyless service response",
        ));
    }

    let mut headers = response_headers_without_transport(&response_headers)?;
    headers.remove(CONTENT_LENGTH);
    if let Some(length) = framing.output_content_length(body.len()) {
        headers.insert(
            CONTENT_LENGTH,
            HeaderValue::from_str(&length.to_string())
                .map_err(|_| GatewayEdgeError::Contract("invalid service response length"))?,
        );
    }
    Ok(GatewayResponse {
        status,
        headers,
        body,
        mailbox_publication: None,
    })
}

/// Owns the Hyper driver for the lifetime of one service request.
struct DriverGuard(JoinHandle<Result<(), hyper::Error>>);

impl Drop for DriverGuard {
    fn drop(&mut self) {
        self.0.abort();
    }
}
