//! Bounded HTTP/1 exchange over one private service connection.

use bytes::Bytes;
use http::{
    HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode, Uri,
    header::{self, CONNECTION, CONTENT_LENGTH, HOST, TRANSFER_ENCODING},
};
use http_body_util::{BodyExt, Full, Limited};
use hyper::client::conn::http1;
use hyper_util::rt::TokioIo;
use std::{collections::HashSet, time::Duration};
use tokio::{task::JoinHandle, time};
use vm_trait::BoxedPrivateServiceConnection;

use crate::{GatewayEdgeError, GatewayLimits, GatewayRequest, GatewayResponse};

const MIN_HTTP1_BUFFER_BYTES: usize = 8 * 1024;
const DEFAULT_WIRE_HEADER_BYTES: usize = 64 * 1024;
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

fn canonical_request(
    request: GatewayRequest,
    policy: ServiceHttpPolicy,
) -> Result<Request<Full<Bytes>>, GatewayEdgeError> {
    if policy.exchange_timeout.is_zero()
        || request.path_and_query.len() > policy.max_path_and_query_bytes
        || request.body.len() > policy.max_request_body_bytes
        || request.headers.len() > policy.max_request_headers
        || !is_canonical_origin_form(&request.path_and_query)
    {
        return Err(GatewayEdgeError::Contract(
            "service request exceeds canonical HTTP bounds",
        ));
    }
    let nominated = connection_nominated_headers(&request.headers)?;
    let mut headers = request.headers;
    let names = headers.keys().cloned().collect::<Vec<_>>();
    for name in names {
        if is_forbidden_request_header(&name)
            || nominated.contains(&name)
            || name == HOST
            || name == CONTENT_LENGTH
        {
            headers.remove(name);
        }
    }
    let host = HeaderValue::try_from(request.trusted.authority.as_str())
        .map_err(|_| GatewayEdgeError::Contract("invalid trusted service authority"))?;
    if host.is_empty() {
        return Err(GatewayEdgeError::Contract(
            "trusted service authority is empty",
        ));
    }
    headers.insert(HOST, host);
    headers.insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&request.body.len().to_string())
            .map_err(|_| GatewayEdgeError::Contract("invalid service request length"))?,
    );
    headers.insert(CONNECTION, HeaderValue::from_static("close"));
    if serialized_header_bytes(&headers) > policy.max_wire_header_bytes {
        return Err(GatewayEdgeError::Contract(
            "service request headers exceed the byte limit",
        ));
    }

    let uri = request
        .path_and_query
        .parse::<Uri>()
        .map_err(|_| GatewayEdgeError::Contract("invalid service request target"))?;
    let mut outbound = Request::new(Full::new(request.body));
    *outbound.method_mut() = request.method;
    *outbound.uri_mut() = uri;
    *outbound.headers_mut() = headers;
    Ok(outbound)
}

fn is_canonical_origin_form(value: &str) -> bool {
    let (path, _) = value.split_once('?').map_or((value, ""), |parts| parts);
    value.starts_with('/')
        && !value.contains('#')
        && !path.contains('%')
        && !path.contains("//")
        && !path.split('/').any(|part| part == "." || part == "..")
        && !value
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
}

fn connection_nominated_headers(
    headers: &HeaderMap,
) -> Result<HashSet<HeaderName>, GatewayEdgeError> {
    let mut nominated = HashSet::new();
    for value in &headers.get_all(CONNECTION) {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid Connection header"))?;
        for token in value.split(',').map(str::trim) {
            if token.is_empty() {
                return Err(GatewayEdgeError::Contract("invalid Connection header"));
            }
            let name = HeaderName::from_bytes(token.as_bytes())
                .map_err(|_| GatewayEdgeError::Contract("invalid Connection header"))?;
            nominated.insert(name);
        }
    }
    Ok(nominated)
}

fn is_forbidden_request_header(name: &HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "proxy-connection"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
            | "expect"
            | "forwarded"
            | "x-forwarded-for"
            | "x-forwarded-host"
            | "x-forwarded-proto"
    )
}

fn serialized_header_bytes(headers: &HeaderMap) -> usize {
    headers
        .iter()
        .map(|(name, value)| name.as_str().len() + value.as_bytes().len() + 4)
        .sum()
}

fn validate_response_headers(
    response: &Response<hyper::body::Incoming>,
    policy: ServiceHttpPolicy,
) -> Result<(), GatewayEdgeError> {
    if response.headers().len() > policy.max_response_headers
        || serialized_header_bytes(response.headers()) > policy.max_wire_header_bytes
    {
        return Err(GatewayEdgeError::Contract(
            "service response headers exceed the limit",
        ));
    }
    Ok(())
}

fn response_headers_without_transport(headers: &HeaderMap) -> Result<HeaderMap, GatewayEdgeError> {
    let nominated = connection_nominated_headers(headers)?;
    let mut headers = headers.clone();
    let names = headers.keys().cloned().collect::<Vec<_>>();
    for name in names {
        if nominated.contains(&name) {
            headers.remove(name);
        }
    }
    for name in [
        CONNECTION,
        HeaderName::from_static("keep-alive"),
        header::PROXY_AUTHENTICATE,
        header::PROXY_AUTHORIZATION,
        HeaderName::from_static("proxy-connection"),
        header::TE,
        header::TRAILER,
        TRANSFER_ENCODING,
        header::UPGRADE,
    ] {
        headers.remove(name);
    }
    Ok(headers)
}

#[derive(Clone, Copy)]
struct ResponseFraming {
    content_length: Option<usize>,
    bodyless: bool,
    preserve_content_length: bool,
}

impl ResponseFraming {
    const fn content_length(self) -> Option<usize> {
        self.content_length
    }

    const fn is_bodyless(self) -> bool {
        self.bodyless
    }

    const fn output_content_length(self, body_length: usize) -> Option<usize> {
        if self.preserve_content_length {
            self.content_length
        } else if self.bodyless {
            None
        } else {
            Some(body_length)
        }
    }
}

fn response_framing(
    response: &Response<hyper::body::Incoming>,
    method: &Method,
) -> Result<ResponseFraming, GatewayEdgeError> {
    let status = response.status();
    if status == StatusCode::SWITCHING_PROTOCOLS {
        return Err(GatewayEdgeError::Contract(
            "service HTTP upgrades are unsupported",
        ));
    }
    let content_length = parse_content_length(response.headers())?;
    let mut encodings = Vec::new();
    let transfer_encoding = response.headers().get_all(TRANSFER_ENCODING);
    for value in &transfer_encoding {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid service response encoding"))?;
        encodings.extend(value.split(',').map(str::trim));
    }
    let bodyless = *method == Method::HEAD
        || status.is_informational()
        || status == StatusCode::NO_CONTENT
        || status == StatusCode::NOT_MODIFIED;
    if bodyless {
        if status == StatusCode::NO_CONTENT
            && (content_length.is_some_and(|length| length != 0) || !encodings.is_empty())
        {
            return Err(GatewayEdgeError::Contract(
                "invalid body framing for a 204 service response",
            ));
        }
        if !encodings.is_empty() && content_length.is_some() {
            return Err(GatewayEdgeError::Contract(
                "ambiguous service response framing",
            ));
        }
        return Ok(ResponseFraming {
            content_length,
            bodyless: true,
            preserve_content_length: *method == Method::HEAD || status == StatusCode::NOT_MODIFIED,
        });
    }
    if !encodings.is_empty() {
        if content_length.is_some()
            || encodings.len() != 1
            || !encodings[0].eq_ignore_ascii_case("chunked")
        {
            return Err(GatewayEdgeError::Contract(
                "ambiguous service response framing",
            ));
        }
        return Ok(ResponseFraming {
            content_length: None,
            bodyless: false,
            preserve_content_length: false,
        });
    }
    content_length.map_or_else(
        || {
            Err(GatewayEdgeError::Contract(
                "close-delimited service responses are unsupported",
            ))
        },
        |length| {
            Ok(ResponseFraming {
                content_length: Some(length),
                bodyless: false,
                preserve_content_length: false,
            })
        },
    )
}

fn parse_content_length(headers: &HeaderMap) -> Result<Option<usize>, GatewayEdgeError> {
    let mut length = None;
    for value in &headers.get_all(CONTENT_LENGTH) {
        let value = value
            .to_str()
            .map_err(|_| GatewayEdgeError::Contract("invalid service response length"))?;
        for part in value.split(',') {
            let parsed = part
                .trim()
                .parse::<u64>()
                .ok()
                .and_then(|value| usize::try_from(value).ok())
                .ok_or(GatewayEdgeError::Contract(
                    "invalid service response length",
                ))?;
            if length.is_some_and(|previous| previous != parsed) {
                return Err(GatewayEdgeError::Contract(
                    "ambiguous service response length",
                ));
            }
            length = Some(parsed);
        }
    }
    Ok(length)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GatewayScheme, TrustedRequestMetadata};
    use std::net::IpAddr;
    use tokio::{io::AsyncReadExt, io::AsyncWriteExt, sync::oneshot};
    use uuid::Uuid;

    fn policy() -> ServiceHttpPolicy {
        ServiceHttpPolicy::from_gateway_limits(GatewayLimits {
            max_request_body_bytes: 128,
            max_response_body_bytes: 128,
            max_request_headers: 16,
            max_response_headers: 16,
            max_path_and_query_bytes: 256,
            execution_timeout: Duration::from_secs(1),
        })
    }

    fn request(body: &'static [u8]) -> GatewayRequest {
        request_with_method(Method::POST, body)
    }

    fn request_with_method(method: Method, body: &'static [u8]) -> GatewayRequest {
        GatewayRequest {
            method,
            path_and_query: String::from("/gateway/store/item?source=test"),
            headers: HeaderMap::new(),
            body: Bytes::from_static(body),
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: String::from("edge.example.test"),
                client_address: IpAddr::from([127, 0, 0, 1]),
                request_id: Uuid::new_v4(),
            },
        }
    }

    async fn read_request_headers(stream: &mut tokio::io::DuplexStream) -> Vec<u8> {
        let mut request = Vec::new();
        let mut buffer = [0_u8; 512];
        loop {
            let count = stream.read(&mut buffer).await.expect("read request");
            assert_ne!(count, 0, "peer closed before request headers");
            request.extend_from_slice(&buffer[..count]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                return request;
            }
        }
    }

    fn connection_pair() -> (BoxedPrivateServiceConnection, tokio::io::DuplexStream) {
        let (client, server) = tokio::io::duplex(16 * 1024);
        (Box::new(client), server)
    }

    #[tokio::test]
    async fn canonicalizes_request_and_strips_dynamic_connection_headers() {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            let request = read_request_headers(&mut server).await;
            let request = String::from_utf8(request).expect("request is HTTP text");
            assert!(request.starts_with("POST /gateway/store/item?source=test HTTP/1.1\r\n"));
            assert!(request.contains("host: edge.example.test\r\n"));
            assert!(request.contains("content-length: 7\r\n"));
            assert!(request.contains("connection: close\r\n"));
            assert!(request.contains("authorization: Bearer app-value\r\n"));
            assert!(!request.contains("host: attacker.example\r\n"));
            assert!(!request.contains("x-forwarded-for"));
            assert!(!request.contains("x-dynamic-hop"));
            server
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\nX-Service: yes\r\n\r\nok",
                )
                .await
                .expect("write response");
        });
        let mut request = request(b"payload");
        request
            .headers
            .insert(HOST, HeaderValue::from_static("attacker.example"));
        request.headers.insert(
            "authorization",
            HeaderValue::from_static("Bearer app-value"),
        );
        request
            .headers
            .insert("x-forwarded-for", HeaderValue::from_static("198.51.100.1"));
        request.headers.insert(
            "connection",
            HeaderValue::from_static("x-dynamic-hop, Upgrade"),
        );
        request
            .headers
            .insert("x-dynamic-hop", HeaderValue::from_static("remove-me"));
        let response = exchange(connection, request, policy())
            .await
            .expect("bounded service exchange");
        server_task.await.expect("server task");
        assert_eq!(response.status, StatusCode::OK);
        assert_eq!(response.body, Bytes::from_static(b"ok"));
        assert_eq!(
            response.headers.get("x-service"),
            Some(&HeaderValue::from_static("yes"))
        );
        assert_eq!(
            response.headers.get(CONTENT_LENGTH),
            Some(&HeaderValue::from_static("2"))
        );
        assert!(!response.headers.contains_key(CONNECTION));
    }

    #[tokio::test]
    async fn preserves_percent_encoded_path_and_url_query() {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            let request = read_request_headers(&mut server).await;
            let request = String::from_utf8(request).expect("request is HTTP text");
            assert!(request.starts_with(
                "POST /gateway/store/item?next=%2Ffoo//bar&url=https://example.test/a?x=1 HTTP/1.1\r\n"
            ));
            server
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await
                .expect("write response");
        });
        let mut request = request(b"");
        request.path_and_query =
            String::from("/gateway/store/item?next=%2Ffoo//bar&url=https://example.test/a?x=1");
        exchange(connection, request, policy())
            .await
            .expect("encoded query is a valid origin form");
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn accepts_chunked_responses_without_trailers() {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            server
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\n\r\n",
                )
                .await
                .expect("write chunked response");
        });
        let response = exchange(connection, request(b""), policy())
            .await
            .expect("chunked response is bounded");
        server_task.await.expect("server task");
        assert_eq!(response.body, Bytes::from_static(b"ok"));
    }

    #[tokio::test]
    async fn accepts_identical_duplicate_content_length() {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            server
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .await
                .expect("write duplicate-length response");
        });
        let response = exchange(connection, request(b""), policy())
            .await
            .expect("identical content lengths are unambiguous");
        server_task.await.expect("server task");
        assert_eq!(response.body, Bytes::from_static(b"ok"));
    }

    #[tokio::test]
    async fn preserves_head_and_not_modified_lengths_but_drops_204_length() {
        let mut body_limited = policy();
        body_limited.max_response_body_bytes = 2;
        for (method, response_bytes, expected_length) in [
            (
                Method::HEAD,
                &b"HTTP/1.1 200 OK\r\nContent-Length: 9\r\nConnection: close\r\n\r\n"[..],
                Some(9),
            ),
            (
                Method::GET,
                &b"HTTP/1.1 304 Not Modified\r\nContent-Length: 9\r\nConnection: close\r\n\r\n"[..],
                Some(9),
            ),
            (
                Method::GET,
                &b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"[..],
                None,
            ),
        ] {
            let (connection, mut server) = connection_pair();
            let server_task = tokio::spawn(async move {
                read_request_headers(&mut server).await;
                server
                    .write_all(response_bytes)
                    .await
                    .expect("write bodyless response");
            });
            let response = exchange(connection, request_with_method(method, b""), body_limited)
                .await
                .expect("bodyless response is bounded");
            server_task.await.expect("server task");
            let actual_length = response.headers.get(CONTENT_LENGTH).map(|value| {
                std::str::from_utf8(value.as_bytes())
                    .expect("length text")
                    .parse::<usize>()
                    .expect("length")
            });
            assert_eq!(actual_length, expected_length);
            assert!(response.body.is_empty());
        }
    }

    #[tokio::test]
    async fn rejects_conflicting_content_length_and_transfer_encoding() {
        let (connection, mut server) = connection_pair();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            let _ = server
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nTransfer-Encoding: chunked\r\n\r\n2\r\nok\r\n0\r\n\r\n",
                )
                .await;
        });
        assert!(matches!(
            exchange(connection, request(b""), policy()).await,
            Err(GatewayEdgeError::Contract(
                "ambiguous service response framing"
            ))
        ));
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn rejects_upgrades_even_without_a_switching_status() {
        for response_bytes in [
            &b"HTTP/1.1 200 OK\r\nUpgrade: websocket\r\nContent-Length: 0\r\n\r\n"[..],
            &b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\n\r\n"[..],
        ] {
            let (connection, mut server) = connection_pair();
            let server_task = tokio::spawn(async move {
                read_request_headers(&mut server).await;
                let _ = server.write_all(response_bytes).await;
            });
            assert!(matches!(
                exchange(connection, request(b""), policy()).await,
                Err(GatewayEdgeError::Contract(_))
            ));
            server_task.await.expect("server task");
        }
    }

    #[tokio::test]
    async fn rejects_trailers_and_close_delimited_responses() {
        for response_bytes in [
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nTrailer: X-Trailer\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\nX-Trailer: value\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n2\r\nok\r\n0\r\nX-Trailer: value\r\n\r\n"[..],
            &b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\nbody"[..],
        ] {
            let (connection, mut server) = connection_pair();
            let server_task = tokio::spawn(async move {
                read_request_headers(&mut server).await;
                let _ = server.write_all(response_bytes).await;
            });
            assert!(matches!(
                exchange(connection, request(b""), policy()).await,
                Err(GatewayEdgeError::Contract(_))
            ));
            server_task.await.expect("server task");
        }
    }

    #[tokio::test]
    async fn enforces_response_body_and_header_limits() {
        for (limits, expected) in [
            (
                ServiceHttpPolicy {
                    max_response_body_bytes: 2,
                    ..policy()
                },
                "body",
            ),
            (policy().with_max_wire_header_bytes(128), "header"),
        ] {
            let (connection, mut server) = connection_pair();
            let server_task = tokio::spawn(async move {
                read_request_headers(&mut server).await;
                server
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nX-One: 1\r\nX-Two: 123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890\r\n\r\ntest",
                    )
                    .await
                    .expect("write oversized response");
            });
            let result = exchange(connection, request(b""), limits).await;
            assert!(
                matches!(result, Err(GatewayEdgeError::Contract(_))),
                "{expected} limit"
            );
            server_task.await.expect("server task");
        }
    }

    #[tokio::test]
    async fn timeout_closes_the_private_stream() {
        let (connection, mut server) = connection_pair();
        let (headers_seen, headers_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            headers_seen.send(()).expect("signal request receipt");
            let mut byte = [0_u8; 1];
            let count = server.read(&mut byte).await.expect("observe stream close");
            assert_eq!(count, 0, "timed-out adapter kept the stream alive");
        });
        let mut short = policy();
        short.exchange_timeout = Duration::from_millis(25);
        let exchange_task = tokio::spawn(exchange(connection, request(b""), short));
        headers_received.await.expect("request reached peer");
        assert!(matches!(
            exchange_task.await.expect("exchange task"),
            Err(GatewayEdgeError::HandlerUnavailable)
        ));
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn caller_cancellation_closes_the_private_stream() {
        let (connection, mut server) = connection_pair();
        let (headers_seen, headers_received) = oneshot::channel();
        let server_task = tokio::spawn(async move {
            read_request_headers(&mut server).await;
            headers_seen.send(()).expect("signal request receipt");
            let mut byte = [0_u8; 1];
            let count = server.read(&mut byte).await.expect("observe stream close");
            assert_eq!(count, 0, "cancelled adapter kept the stream alive");
        });
        let exchange_task = tokio::spawn(exchange(connection, request(b""), policy()));
        headers_received.await.expect("request reached peer");
        exchange_task.abort();
        assert!(
            exchange_task
                .await
                .expect_err("exchange was cancelled")
                .is_cancelled()
        );
        server_task.await.expect("server task");
    }

    #[tokio::test]
    async fn rejects_pathological_public_policy_values() {
        let (connection, _server) = connection_pair();
        let mut no_headers = policy();
        no_headers.max_response_headers = 0;
        assert!(matches!(
            exchange(connection, request(b""), no_headers).await,
            Err(GatewayEdgeError::Contract(_))
        ));

        let (connection, _server) = connection_pair();
        let mut unbounded_timeout = policy();
        unbounded_timeout.exchange_timeout = Duration::MAX;
        assert!(matches!(
            exchange(connection, request(b""), unbounded_timeout).await,
            Err(GatewayEdgeError::Contract(_))
        ));
    }
}
