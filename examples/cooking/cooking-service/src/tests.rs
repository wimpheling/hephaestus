use std::{
    io::{ErrorKind, Read, Write},
    net::{Shutdown, TcpListener, TcpStream},
    thread,
    time::{Duration, Instant},
};

use super::{
    isolation::parse_isolation_ports,
    routing::response_for,
    server::{read_request_with_budget, serve_connection},
    types::{IsolationPorts, StartupIdentity},
};

fn test_identity() -> StartupIdentity {
    StartupIdentity {
        pid: 7,
        value: String::from("7-test-startup"),
    }
}

fn round_trip(request: &[u8], identity: &StartupIdentity) -> Vec<u8> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener");
    let address = listener.local_addr().expect("test address");
    let identity = identity.clone();
    let worker = thread::spawn(move || {
        let (stream, _) = listener.accept().expect("test connection");
        serve_connection(stream, &identity).expect("serve test request");
    });
    let mut client = TcpStream::connect(address).expect("test client");
    client.write_all(request).expect("write test request");
    client
        .shutdown(Shutdown::Write)
        .expect("finish test request");
    let mut response = Vec::new();
    client
        .read_to_end(&mut response)
        .expect("read test response");
    worker.join().expect("test worker");
    response
}

#[test]
fn repeated_identity_requests_keep_one_process_identity() {
    let identity = test_identity();
    for path in ["/service/identity", "/gateway/service/identity"] {
        let request = format!("GET {path} HTTP/1.1\r\nHost: service\r\n\r\n");
        let first = round_trip(request.as_bytes(), &identity);
        let second = round_trip(request.as_bytes(), &identity);
        assert_eq!(first, second);
        assert!(
            first
                .windows(b"7-test-startup".len())
                .any(|window| { window == b"7-test-startup" })
        );
    }
}

#[test]
fn probes_and_public_routes_have_bounded_http_responses() {
    let identity = test_identity();
    for (path, expected) in [
            ("/readyz", b"ready".as_slice()),
            ("/healthz", b"healthy".as_slice()),
            ("/", b"cooking service".as_slice()),
            ("/service", b"cooking service".as_slice()),
            ("/gateway/service", b"cooking service".as_slice()),
            (
                "/gateway/service/metadata",
                b"{\"host_matches_expected\":false,\"forwarded_present\":false,\"x_forwarded_for_present\":false,\"x_forwarded_host_present\":false,\"x_forwarded_proto_present\":false}".as_slice(),
            ),
            (
                "/gateway/service/identity",
                b"\"startup_id\":\"7-test-startup\"".as_slice(),
            ),
        ] {
            let request = format!("GET {path} HTTP/1.1\r\nHost: service\r\n\r\n");
            let response = round_trip(request.as_bytes(), &identity);
            assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
            if path.ends_with("identity") {
                assert!(
                    response
                        .windows(expected.len())
                        .any(|window| window == expected)
                );
            } else {
                assert!(response.ends_with(expected));
            }
            assert!(
                response
                    .windows(b"Connection: close".len())
                    .any(|window| { window.eq_ignore_ascii_case(b"Connection: close") })
            );
        }
}

#[test]
fn isolation_query_accepts_only_distinct_non_service_ports() {
    assert_eq!(
        parse_isolation_ports("/gateway/service/isolation?admin_port=2019&public_port=18080"),
        Some(IsolationPorts {
            admin: 2019,
            public: 18080,
        })
    );
    for query in [
        "/service/isolation?admin_port=8080&public_port=18080",
        "/service/isolation?admin_port=2019&public_port=2019",
        "/service/isolation?admin_port=2019&public_port=18080&host=127.0.0.1",
        "/service/isolation?admin_port=not-a-port&public_port=18080",
        "/service/isolation?admin_port=+2019&public_port=18080",
        "/service/isolation?admin_port=65536&public_port=18080",
        "/service/isolation?admin_port=0&public_port=18080",
        "/service/isolation?admin_port=2019",
    ] {
        assert_eq!(parse_isolation_ports(query), None, "query: {query}");
    }
}

#[test]
fn isolation_routes_reject_arbitrary_destinations() {
    let identity = test_identity();
    let response = response_for(
            b"GET /gateway/service/isolation?admin_port=2019&public_port=18080&host=169.254.169.254 HTTP/1.1\r\nHost: service\r\n\r\n",
            &identity,
        );
    assert_eq!(response.status, 400);
}

#[test]
fn metadata_reports_only_case_insensitive_forwarding_presence() {
    let identity = test_identity();
    let response = response_for(
            b"GET /service/metadata HTTP/1.1\r\nHoSt: gateway.golden.invalid\r\nfOrWaRdEd: for=attacker\r\nX-FORWARDED-FOR: 192.0.2.7\r\nX-Forwarded-Host: attacker.invalid\r\nX-forwarded-PROTO: http\r\nX-Secret: do-not-report\r\n\r\n",
            &identity,
        );
    assert_eq!(response.status, 200);
    let body = String::from_utf8(response.body).expect("metadata JSON UTF-8");
    assert!(body.contains("\"host_matches_expected\":true"));
    assert!(body.contains("\"forwarded_present\":true"));
    assert!(body.contains("\"x_forwarded_for_present\":true"));
    assert!(body.contains("\"x_forwarded_host_present\":true"));
    assert!(body.contains("\"x_forwarded_proto_present\":true"));
    assert!(!body.contains("attacker"));
    assert!(!body.contains("do-not-report"));
}

#[test]
fn metadata_rejects_duplicate_host_headers() {
    let identity = test_identity();
    let response = response_for(
            b"GET /gateway/service/metadata HTTP/1.1\r\nHost: gateway.golden.invalid\r\nhOsT: attacker.invalid\r\n\r\n",
            &identity,
        );
    assert_eq!(response.status, 400);
}

#[test]
fn rejects_body_and_protocol_upgrade_requests() {
    let identity = test_identity();
    for request in [
        b"GET / HTTP/1.1\r\nHost: service\r\nContent-Length: 1\r\n\r\nx".as_slice(),
        b"GET / HTTP/1.1\r\nHost: service\r\nUpgrade: websocket\r\n\r\n".as_slice(),
    ] {
        let response = round_trip(request, &identity);
        assert!(response.starts_with(b"HTTP/1.1 400 Bad Request\r\n"));
    }
}

#[test]
fn trickled_headers_share_one_absolute_read_deadline() {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("test listener");
    let address = listener.local_addr().expect("test address");
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().expect("test connection");
        read_request_with_budget(&mut stream, Duration::from_millis(100))
    });
    let mut client = TcpStream::connect(address).expect("test client");
    let started = Instant::now();
    client
        .write_all(b"GET / HTTP/1.1\r\n")
        .expect("write first header chunk");
    thread::sleep(Duration::from_millis(70));
    client
        .write_all(b"Host: service\r\n")
        .expect("write second header chunk");
    let result = worker.join().expect("test worker");
    assert!(matches!(
        result.expect_err("trickled request must time out").kind(),
        ErrorKind::TimedOut | ErrorKind::WouldBlock
    ));
    assert!(started.elapsed() < Duration::from_millis(150));
}
