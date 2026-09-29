use super::*;
use crate::{
    GatewayEdgeError, GatewayLimits, GatewayRequest, GatewayScheme, TrustedRequestMetadata,
};
use bytes::Bytes;
use http::{
    HeaderMap, HeaderValue, Method, StatusCode,
    header::{CONNECTION, CONTENT_LENGTH, HOST},
};
use std::{net::IpAddr, time::Duration};
use tokio::{io::AsyncReadExt, io::AsyncWriteExt, sync::oneshot};
use uuid::Uuid;
use vm_trait::BoxedPrivateServiceConnection;

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

#[path = "service_http_tests/request_behavior.rs"]
mod request_behavior;
#[path = "service_http_tests/response_behavior.rs"]
mod response_behavior;
