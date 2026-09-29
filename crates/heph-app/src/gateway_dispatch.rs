use super::{Arc, Duration, SocketAddr, Uuid};
use axum::{
    body::Body,
    extract::{ConnectInfo, State},
    http::Request,
    response::Response,
};
use bytes::Bytes;
use gateway_edge::{
    GatewayLimits, GatewayRequest, GatewayRequestDispatcher, GatewayScheme, TrustedRequestMetadata,
    UNTRUSTED_FORWARDING_HEADERS,
};

const MAX_PRIVATE_GATEWAY_REQUEST_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone)]
pub struct PrivateGatewayDispatcherState {
    pub dispatcher: Arc<dyn GatewayRequestDispatcher>,
    pub public_authority: String,
}

pub const fn gateway_limits() -> GatewayLimits {
    GatewayLimits {
        max_request_body_bytes: MAX_PRIVATE_GATEWAY_REQUEST_BYTES,
        max_response_body_bytes: MAX_PRIVATE_GATEWAY_REQUEST_BYTES,
        max_request_headers: 128,
        max_response_headers: 128,
        max_path_and_query_bytes: 8 * 1024,
        execution_timeout: Duration::from_secs(30),
    }
}

pub async fn private_gateway_dispatch(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<PrivateGatewayDispatcherState>,
    request: Request<Body>,
) -> Response {
    if !peer.ip().is_loopback() {
        return gateway_http_response(
            http::StatusCode::FORBIDDEN,
            http::HeaderMap::new(),
            Bytes::new(),
        );
    }
    let path_and_query = request
        .uri()
        .path_and_query()
        .map_or_else(|| String::from("/"), ToString::to_string);
    let method = request.method().clone();
    let mut headers = request.headers().clone();
    for name in UNTRUSTED_FORWARDING_HEADERS {
        headers.remove(name);
    }
    for name in [
        http::header::CONNECTION,
        http::header::PROXY_AUTHENTICATE,
        http::header::PROXY_AUTHORIZATION,
        http::header::TE,
        http::header::TRAILER,
        http::header::TRANSFER_ENCODING,
        http::header::UPGRADE,
    ] {
        headers.remove(name);
    }
    headers.remove("keep-alive");
    let Ok(body) =
        axum::body::to_bytes(request.into_body(), MAX_PRIVATE_GATEWAY_REQUEST_BYTES).await
    else {
        return gateway_http_response(
            http::StatusCode::PAYLOAD_TOO_LARGE,
            http::HeaderMap::new(),
            Bytes::new(),
        );
    };
    let response = state
        .dispatcher
        .dispatch(GatewayRequest {
            method,
            path_and_query,
            headers,
            body,
            trusted: TrustedRequestMetadata {
                scheme: GatewayScheme::Https,
                authority: state.public_authority,
                client_address: peer.ip(),
                request_id: Uuid::new_v4(),
            },
        })
        .await
        .response;
    gateway_http_response(response.status, response.headers, response.body)
}

fn gateway_http_response(
    status: http::StatusCode,
    headers: http::HeaderMap,
    body: Bytes,
) -> Response {
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}
