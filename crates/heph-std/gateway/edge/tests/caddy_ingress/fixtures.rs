use async_trait::async_trait;
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayLimits, GatewayProviderResponse,
    GatewayRequest, GatewayRequestDispatcher, GatewayRouteBinding,
};
use http::Method;
use std::{collections::BTreeSet, env, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::timeout,
};

pub struct NoopDispatcher;

#[async_trait]
impl GatewayRequestDispatcher for NoopDispatcher {
    async fn dispatch(&self, _: GatewayRequest) -> GatewayProviderResponse {
        unreachable!("Caddy integration test uses its private TCP dispatcher fixture")
    }
}

pub fn desired() -> GatewayDesiredConfiguration {
    desired_path("proof")
}

pub fn desired_path(path_prefix: &str) -> GatewayDesiredConfiguration {
    GatewayDesiredConfiguration {
        revision: GatewayConfigRevision::new(),
        routes: vec![GatewayRouteBinding {
            route_id: uuid::Uuid::new_v4(),
            exposure: gateway_domain::Exposure::Public,
            gateway_revision_id: uuid::Uuid::new_v4(),
            path_prefix: String::from(path_prefix),
            methods: BTreeSet::from([Method::POST]),
            limits: GatewayLimits {
                max_request_body_bytes: 1024,
                max_response_body_bytes: 1024,
                max_request_headers: 16,
                max_response_headers: 16,
                max_path_and_query_bytes: 256,
                execution_timeout: Duration::from_secs(1),
            },
        }],
    }
}

pub async fn serve_ui_requests(listener: TcpListener, count: usize) {
    for _ in 0..count {
        let (mut stream, _) = listener.accept().await.expect("accept Caddy UI relay");
        let request = timeout(Duration::from_secs(5), read_headers(&mut stream))
            .await
            .expect("UI request headers before deadline");
        let request_line = request.lines().next().expect("Caddy request line");
        assert!(
            request_line.starts_with("GET /"),
            "unexpected UI request: {request_line}"
        );
        let host = request
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("host").then_some(value.trim())
            })
            .expect("Caddy supplied the UI Host header");
        let (status, body) = if matches!(host, "ui.example" | "deep.ui.example") {
            ("200 OK", "ui-upstream")
        } else {
            ("404 Not Found", "ui-upstream-unknown")
        };
        let response = format!(
            "HTTP/1.1 {status}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream
            .write_all(response.as_bytes())
            .await
            .expect("write UI response");
    }
}

async fn read_headers(stream: &mut tokio::net::TcpStream) -> String {
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1_024];
    loop {
        let bytes = stream.read(&mut buffer).await.expect("read Caddy request");
        assert_ne!(bytes, 0, "Caddy closed before request headers");
        request.extend_from_slice(&buffer[..bytes]);
        if request.windows(4).any(|window| window == b"\r\n\r\n") {
            return String::from_utf8_lossy(&request).into_owned();
        }
    }
}

pub async fn serve_requests(listener: TcpListener, count: usize) {
    for _ in 0..count {
        timeout(Duration::from_secs(5), serve_one(&listener))
            .await
            .expect("dispatcher request before deadline");
    }
}

async fn serve_one(listener: &TcpListener) {
    let (mut stream, _) = listener.accept().await.expect("accept Caddy relay");
    let mut request = Vec::new();
    let mut buffer = [0_u8; 1_024];
    loop {
        let bytes = stream.read(&mut buffer).await.expect("read Caddy relay");
        assert_ne!(bytes, 0, "Caddy closed before completing its request");
        request.extend_from_slice(&buffer[..bytes]);
        let Some(headers_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let headers = String::from_utf8_lossy(&request[..headers_end + 4]);
        let content_length = headers
            .lines()
            .find_map(|line| line.strip_prefix("Content-Length: "))
            .expect("Caddy supplied a content length")
            .parse::<usize>()
            .expect("numeric Caddy content length");
        if request.len() >= headers_end + 4 + content_length {
            break;
        }
    }
    let request = String::from_utf8_lossy(&request);
    assert!(request.starts_with("POST /gateway/"));
    stream
        .write_all(
            b"HTTP/1.1 201 Created\r\nContent-Type: text/plain\r\nX-Gateway-Proof: shared-caddy\r\nContent-Length: 22\r\nConnection: close\r\n\r\ngateway-caddy-response",
        )
        .await
        .expect("write dispatcher response");
}

pub fn caddy_configuration_with_ui_slot(admin_url: &str) -> Vec<u8> {
    let mut configuration: serde_json::Value =
        serde_json::from_slice(&caddy_configuration(admin_url)).expect("base Caddy JSON");
    let routes = configuration
        .pointer_mut("/apps/http/servers/shared/routes")
        .and_then(serde_json::Value::as_array_mut)
        .expect("shared Caddy routes");
    routes.insert(
        0,
        serde_json::json!({
            "group": "hephaestus.ui",
            "handle": [{ "handler": "subroute", "routes": [] }]
        }),
    );
    serde_json::to_vec(&configuration).expect("UI Caddy JSON")
}

pub fn caddy_configuration(admin_url: &str) -> Vec<u8> {
    let admin = reqwest::Url::parse(admin_url).expect("admin URL");
    let admin_listen = format!(
        "{}:{}",
        admin.host_str().expect("admin host"),
        admin.port().expect("admin port")
    );
    serde_json::json!({
        "admin": { "listen": admin_listen },
        "apps": { "http": { "servers": { "shared": {
            "listen": [env::var("HEPHAESTUS_CADDY_TEST_LISTEN").expect("Caddy public listen address")],
            "routes": [
                { "match": [{ "path": ["/platform/*"] }], "handle": [{ "handler": "static_response", "body": "platform-owned" }] },
                { "group": "hephaestus.gateway", "handle": [{ "handler": "subroute", "routes": [] }] },
                { "handle": [{ "handler": "static_response", "status_code": 404 }] }
            ]
        } } } }
    })
    .to_string()
    .into_bytes()
}
