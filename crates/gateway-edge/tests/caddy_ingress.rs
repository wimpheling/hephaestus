//! Opt-in shared-Caddy ingress proof.
//!
//! `scripts/test-gateway-caddy-smoke.sh` owns the disposable Caddy container
//! and supplies its loopback-only admin and public listener URLs.

use async_trait::async_trait;
use gateway_edge::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayLimits, GatewayProvider,
    GatewayProviderResponse, GatewayRequest, GatewayRequestDispatcher, GatewayRouteBinding,
    LocalCaddyAdministration, LocalCaddyConfigurationTemplate, LocalCaddyGatewayProvider,
};
use http::Method;
use std::{collections::BTreeSet, env, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

#[tokio::test(flavor = "multi_thread")]
async fn preserves_platform_routes_and_removes_tombstoned_gateway_routes() {
    let Ok(admin_url) = env::var("HEPHAESTUS_CADDY_TEST_ADMIN_URL") else {
        return;
    };
    let public_url = env::var("HEPHAESTUS_CADDY_TEST_PUBLIC_URL")
        .expect("Caddy public listener URL supplied with admin URL");

    let upstream = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind private dispatcher fixture");
    let upstream_address = upstream.local_addr().expect("read dispatcher address");
    let responder = tokio::spawn(serve_requests(upstream, 2));
    let template = LocalCaddyConfigurationTemplate::new(
        &caddy_configuration(&admin_url),
        String::from("shared"),
    )
    .expect("valid complete shared Caddy configuration");
    let provider = LocalCaddyGatewayProvider::new(
        LocalCaddyAdministration::new(&admin_url).expect("loopback Caddy admin client"),
        NoopDispatcher,
    )
    .with_dispatcher_upstream(upstream_address.to_string())
    .with_configuration_template(template);
    provider
        .reconcile(&desired())
        .await
        .expect("atomically merge the gateway subroute into shared Caddy");

    let response = reqwest::Client::new()
        .post(format!("{public_url}/gateway/proof?source=caddy"))
        .header("x-forwarded-for", "spoofed")
        .body("gateway-caddy-request")
        .send()
        .await
        .expect("Caddy public gateway request");
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    assert_eq!(
        response.headers().get("x-gateway-proof"),
        Some(&reqwest::header::HeaderValue::from_static("shared-caddy"))
    );
    assert_eq!(
        response.bytes().await.expect("bounded response body"),
        "gateway-caddy-response"
    );

    provider
        .reconcile(&desired_path("changed"))
        .await
        .expect("atomically update the gateway subroute");
    let old = reqwest::get(format!("{public_url}/gateway/proof"))
        .await
        .expect("removed route response after gateway update");
    assert_eq!(old.status(), reqwest::StatusCode::NOT_FOUND);
    let updated = reqwest::Client::new()
        .post(format!("{public_url}/gateway/changed"))
        .body("updated-gateway-request")
        .send()
        .await
        .expect("updated Caddy gateway request");
    assert_eq!(updated.status(), reqwest::StatusCode::CREATED);
    let platform = reqwest::get(format!("{public_url}/platform/status"))
        .await
        .expect("platform route survives gateway reconciliation");
    assert_eq!(platform.status(), reqwest::StatusCode::OK);
    assert_eq!(
        platform.text().await.expect("platform body"),
        "platform-owned"
    );

    provider
        .reconcile(&GatewayDesiredConfiguration {
            revision: GatewayConfigRevision::new(),
            routes: Vec::new(),
        })
        .await
        .expect("atomically remove tombstoned gateway routes");
    let removed = reqwest::get(format!("{public_url}/gateway/proof"))
        .await
        .expect("tombstoned gateway route response");
    assert_eq!(removed.status(), reqwest::StatusCode::NOT_FOUND);
    let platform_after = reqwest::get(format!("{public_url}/platform/status"))
        .await
        .expect("platform route after tombstone");
    assert_eq!(platform_after.status(), reqwest::StatusCode::OK);
    responder.await.expect("private dispatcher task");
}

struct NoopDispatcher;

#[async_trait]
impl GatewayRequestDispatcher for NoopDispatcher {
    async fn dispatch(&self, _: GatewayRequest) -> GatewayProviderResponse {
        unreachable!("Caddy integration test uses its private TCP dispatcher fixture")
    }
}

fn desired() -> GatewayDesiredConfiguration {
    desired_path("proof")
}

fn desired_path(path_prefix: &str) -> GatewayDesiredConfiguration {
    GatewayDesiredConfiguration {
        revision: GatewayConfigRevision::new(),
        routes: vec![GatewayRouteBinding {
            route_id: uuid::Uuid::new_v4(),
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

async fn serve_requests(listener: TcpListener, count: usize) {
    for _ in 0..count {
        serve_one(&listener).await;
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

fn caddy_configuration(admin_url: &str) -> Vec<u8> {
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
