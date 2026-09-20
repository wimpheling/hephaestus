//! Semantic parser coverage for the managed Cooking reference UI.

use agent_config::ui::{UiContent, validate_repository_uis_against_gateways};
use agent_config::{
    BuildArtifactKind, NetworkProfile, RepositoryGatewaysConfig, parse, parse_repository_gateways,
    parse_repository_uis,
};
use gateway_domain::{Exposure, HttpMethod};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const AGENT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/agent.toml"
));
const GATEWAYS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/heph.gateways.toml"
));
const UI: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../examples/cooking/cooking-reference-service-ui/heph.ui.toml"
));

#[test]
fn managed_reference_fixture_binds_authenticated_service_and_runtime_files() {
    assert_agent_manifest();
    let gateway_config = parse_gateway_manifest();
    assert_ui_manifest(&gateway_config);
}

fn assert_agent_manifest() {
    let parsed_agent = parse(AGENT);
    assert!(
        parsed_agent.diagnostics.is_empty(),
        "{:?}",
        parsed_agent.diagnostics
    );
    let agent = parsed_agent.config.expect("valid managed reference agent");
    assert_eq!(
        agent.agent.key.as_deref(),
        Some("cooking-reference-service-ui")
    );
    assert_eq!(agent.guest.command, "bin/reference-ui-service");
    assert!(!agent.workspace.mount);
    assert!(!agent.state_volume.enabled);
    assert!(matches!(agent.network.profile, NetworkProfile::Disabled));

    let build = agent.build.expect("version-2 build configuration");
    assert!(matches!(build.network.profile, NetworkProfile::Disabled));
    let artifacts = build
        .artifacts
        .iter()
        .map(|artifact| {
            (
                &artifact.path,
                artifact.kind,
                artifact.media_type.as_deref(),
            )
        })
        .collect::<Vec<_>>();
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/reference-ui-service"
            && *kind == BuildArtifactKind::Executable
            && *media_type == Some("text/x-python")
    }));
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/index.html"
            && *kind == BuildArtifactKind::File
            && *media_type == Some("text/html")
    }));
    assert!(artifacts.iter().any(|(path, kind, media_type)| {
        *path == "bin/heph-ui-kit-v1.0.0.css"
            && *kind == BuildArtifactKind::File
            && *media_type == Some("text/css")
    }));
}

fn parse_gateway_manifest() -> RepositoryGatewaysConfig {
    let parsed_gateways = parse_repository_gateways(GATEWAYS);
    assert!(
        parsed_gateways.diagnostics.is_empty(),
        "{:?}",
        parsed_gateways.diagnostics
    );
    let gateway_config = parsed_gateways.config.expect("valid gateway manifest");
    let gateway = gateway_config.gateways.first().expect("managed UI gateway");
    assert_eq!(gateway.name, "cooking-reference-service-ui");
    assert_eq!(gateway.agent_name, "cooking-reference-service-ui");
    assert_eq!(gateway.handler_contract, "http.service.v1");
    assert_eq!(gateway.exposure, Exposure::HephAuthenticated);
    let service = gateway.service.as_ref().expect("service settings");
    assert_eq!(service.loopback_port, 8080);
    assert_eq!(service.readiness_path, "/readyz");
    assert_eq!(service.health_path, "/healthz");
    assert_eq!(gateway.routes.len(), 1);
    assert_eq!(gateway.routes[0].path, "/reference");
    assert_eq!(gateway.routes[0].methods, vec![HttpMethod::Get]);
    gateway_config
}

fn assert_ui_manifest(gateway_config: &RepositoryGatewaysConfig) {
    let parsed_ui = parse_repository_uis(UI);
    assert!(
        parsed_ui.diagnostics.is_empty(),
        "{:?}",
        parsed_ui.diagnostics
    );
    let ui = parsed_ui.config.expect("valid managed UI manifest");
    assert_eq!(ui.uis.len(), 1);
    let declaration = &ui.uis[0];
    assert_eq!(declaration.key.as_str(), "managed-reference");
    assert_eq!(declaration.apis.len(), 5);
    let api_tuples = declaration
        .apis
        .iter()
        .map(|api| {
            (
                api.key.as_str(),
                api.gateway_name.as_str(),
                api.method,
                api.route.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        api_tuples,
        vec![
            (
                "header-policy",
                "cooking-reference-service-ui",
                HttpMethod::Get,
                "/reference/header-policy"
            ),
            (
                "identity",
                "cooking-reference-service-ui",
                HttpMethod::Get,
                "/reference/identity"
            ),
            (
                "probe-location",
                "cooking-reference-service-ui",
                HttpMethod::Get,
                "/reference/probe-location"
            ),
            (
                "probe-refresh",
                "cooking-reference-service-ui",
                HttpMethod::Get,
                "/reference/probe-refresh"
            ),
            (
                "probe-set-cookie",
                "cooking-reference-service-ui",
                HttpMethod::Get,
                "/reference/probe-set-cookie"
            ),
        ]
    );
    match &declaration.content {
        UiContent::ManagedService {
            gateway_name,
            route,
            entrypoint,
        } => {
            assert_eq!(gateway_name.as_str(), "cooking-reference-service-ui");
            assert_eq!(route.as_str(), "/reference");
            assert_eq!(entrypoint.as_str(), "index.html");
        }
        UiContent::Static { .. } => panic!("managed fixture parsed as static content"),
    }
    assert!(validate_repository_uis_against_gateways(&ui, Some(gateway_config)).is_empty());
}

#[test]
fn local_reference_service_probes_guest_header_and_response_policies() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("reserve local probe port");
    let port = listener.local_addr().expect("read local probe port").port();
    drop(listener);
    let service = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/cooking/cooking-reference-service-ui/reference-ui-service.py");
    let child = Command::new("python3")
        .arg(service)
        .env("HEPHAESTUS_REFERENCE_SERVICE_PORT", port.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("start reference service with explicit python3");
    let mut child = ChildGuard { child };
    let address = format!("127.0.0.1:{port}");
    wait_for_service(&mut child.child, &address);

    let (_, _, body) = http_get(
        &address,
        "/reference/header-policy",
        &[
            ("Authorization", "Bearer redacted-test-value"),
            ("Cookie", "redacted-test-cookie"),
            ("Forwarded", "for=redacted"),
            ("X-Forwarded-For", "redacted"),
            ("x-FoRwArDeD-Proto", "https"),
        ],
    );
    let policy: BTreeMap<String, bool> = serde_json::from_str(&body).expect("policy JSON");
    assert_eq!(
        policy,
        BTreeMap::from([
            (String::from("authorization_absent"), false),
            (String::from("cookie_absent"), false),
            (String::from("forwarded_absent"), false),
            (String::from("x_forwarded_absent"), false),
        ])
    );

    let (_, headers, body) = http_get(&address, "/gateway/reference/header-policy", &[]);
    let policy: BTreeMap<String, bool> = serde_json::from_str(&body).expect("clean policy JSON");
    assert!(policy.values().all(|value| *value));
    assert!(!headers.contains_key("set-cookie"));

    let (status, headers, _) = http_get(&address, "/reference/probe-set-cookie", &[]);
    assert_eq!(status, 200);
    assert_eq!(
        headers.get("set-cookie").map(String::as_str),
        Some("__Host-heph_guest_probe=discarded; Secure; HttpOnly; Path=/")
    );
    let (status, headers, _) = http_get(&address, "/reference/probe-location", &[]);
    assert_eq!(status, 302);
    assert_eq!(
        headers.get("location").map(String::as_str),
        Some("/reference/identity")
    );
    let (status, headers, _) = http_get(&address, "/reference/probe-refresh", &[]);
    assert_eq!(status, 200);
    assert_eq!(
        headers.get("refresh").map(String::as_str),
        Some("0; url=/reference/identity")
    );
}

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for_service(child: &mut Child, address: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(status) = child.try_wait() {
            assert!(
                status.is_none(),
                "reference service exited before local probe"
            );
        }
        if let Ok(mut stream) = TcpStream::connect(address) {
            stream
                .set_read_timeout(Some(Duration::from_millis(250)))
                .expect("set readiness probe timeout");
            stream
                .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .expect("write readiness probe");
            let mut response = Vec::new();
            if stream.read_to_end(&mut response).is_ok() && response.starts_with(b"HTTP/1.1 200") {
                return;
            }
        }
        assert!(Instant::now() < deadline, "reference service did not start");
        thread::sleep(Duration::from_millis(25));
    }
}

fn http_get(
    address: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, BTreeMap<String, String>, String) {
    let mut stream = TcpStream::connect(address).expect("connect local reference service");
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .expect("set local probe timeout");
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n"
    )
    .expect("write local probe request");
    for (name, value) in headers {
        write!(stream, "{name}: {value}\r\n").expect("write local probe header");
    }
    write!(stream, "\r\n").expect("finish local probe request");
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .expect("read local probe response");
    let response = String::from_utf8(response).expect("local probe response is UTF-8");
    let (header_text, body) = response
        .split_once("\r\n\r\n")
        .expect("HTTP response headers");
    let mut lines = header_text.lines();
    let status = lines
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .expect("HTTP response status");
    let response_headers = lines
        .filter_map(|line| line.split_once(": "))
        .map(|(name, value)| (name.to_ascii_lowercase(), value.to_owned()))
        .collect();
    (status, response_headers, body.to_owned())
}
