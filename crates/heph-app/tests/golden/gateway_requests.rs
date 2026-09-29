use super::*;

/// Sends two public requests through the real Caddy/daemon path and proves
/// they reached one long-lived guest process rather than two per-request VMs.
pub struct GatewayServiceRequestProof {
    pub pid: u64,
    pub startup_id: String,
}

pub async fn exercise_gateway_service_requests(public_url: &str) -> GatewayServiceRequestProof {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("bounded persistent-service client");
    let identity_url = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(&identity_url)
        .send()
        .await
        .expect("first public persistent-service request")
        .error_for_status()
        .expect("first persistent-service request succeeds")
        .bytes()
        .await
        .expect("read first persistent-service identity");
    let second = client
        .get(&identity_url)
        .send()
        .await
        .expect("second public persistent-service request")
        .error_for_status()
        .expect("second persistent-service request succeeds")
        .bytes()
        .await
        .expect("read second persistent-service identity");
    let first: serde_json::Value =
        serde_json::from_slice(&first).expect("first service identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second service identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("first service response startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .expect("second service response startup identity");
    assert_eq!(first_startup, second_startup);
    let first_pid = first
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("first service response process identity");
    assert!(first_pid > 0, "first service response PID must be positive");
    let second_pid = second
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("second service response process identity");
    assert_eq!(first_pid, second_pid);
    let first_count = first
        .get("request_count")
        .and_then(serde_json::Value::as_u64)
        .expect("first service response request count");
    let second_count = second
        .get("request_count")
        .and_then(serde_json::Value::as_u64)
        .expect("second service response request count");
    assert!(second_count > first_count);
    println!(
        "persistent-service-public identity_equal=true pid={first_pid} startup_id={first_startup} request_count={first_count}->{second_count}"
    );
    GatewayServiceRequestProof {
        pid: first_pid,
        startup_id: first_startup.to_owned(),
    }
}

/// Sends two identity requests to the published cooking service. Its small
/// sample binary reports only PID and startup identity, so this proof keeps
/// the request-count assertion private to the seeded integration fixture.
pub async fn exercise_published_cooking_service_identity(
    public_url: &str,
) -> GatewayServiceRequestProof {
    let client = published_cooking_service_client();
    let identity_url = format!("{public_url}/gateway/service/identity");
    let first = client
        .get(&identity_url)
        .send()
        .await
        .expect("first published cooking-service identity request")
        .error_for_status()
        .expect("first published cooking-service identity succeeds")
        .bytes()
        .await
        .expect("read first published cooking-service identity");
    let second = client
        .get(&identity_url)
        .send()
        .await
        .expect("second published cooking-service identity request")
        .error_for_status()
        .expect("second published cooking-service identity succeeds")
        .bytes()
        .await
        .expect("read second published cooking-service identity");
    let first: serde_json::Value =
        serde_json::from_slice(&first).expect("first published cooking-service identity JSON");
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second published cooking-service identity JSON");
    let first_startup = first
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .expect("first published cooking-service startup identity");
    let second_startup = second
        .get("startup_id")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .expect("second published cooking-service startup identity");
    assert_eq!(first_startup, second_startup);
    let first_pid = first
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("first published cooking-service PID");
    assert!(
        first_pid > 0,
        "published cooking-service PID must be positive"
    );
    let second_pid = second
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .expect("second published cooking-service PID");
    assert_eq!(first_pid, second_pid);
    GatewayServiceRequestProof {
        pid: first_pid,
        startup_id: first_startup.to_owned(),
    }
}

pub fn published_cooking_service_client() -> reqwest::Client {
    let mut builder = reqwest::Client::builder().timeout(Duration::from_secs(10));
    if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        let ca_path =
            env::var("HEPHAESTUS_CADDY_TEST_CA_CERT").expect("joined Caddy TLS fixture CA path");
        let ca_pem = fs::read(&ca_path).expect("read joined Caddy TLS fixture CA");
        let certificate =
            reqwest::Certificate::from_pem(&ca_pem).expect("parse joined Caddy TLS fixture CA");
        builder = builder
            .tls_built_in_root_certs(false)
            .add_root_certificate(certificate);
    }
    builder
        .build()
        .expect("bounded published cooking-service client")
}

pub async fn exercise_published_cooking_service_metadata(public_url: &str) {
    let client = published_cooking_service_client();
    let metadata_url = format!("{public_url}/gateway/service/metadata");
    for (label, request) in [
        ("normal", client.get(&metadata_url)),
        (
            "forged forwarding",
            client
                .get(&metadata_url)
                .header(reqwest::header::HOST, "attacker.golden.invalid")
                .header("Forwarded", "for=198.51.100.7;host=attacker.golden.invalid")
                .header("X-Forwarded-For", "198.51.100.7")
                .header("X-Forwarded-Host", "attacker.golden.invalid")
                .header("X-Forwarded-Proto", "http"),
        ),
    ] {
        let response = request
            .send()
            .await
            .unwrap_or_else(|error| panic!("{label} published metadata request: {error}"))
            .error_for_status()
            .unwrap_or_else(|error| panic!("{label} published metadata request succeeds: {error}"))
            .bytes()
            .await
            .unwrap_or_else(|error| panic!("read {label} published metadata response: {error}"));
        let metadata: serde_json::Value = serde_json::from_slice(&response)
            .unwrap_or_else(|error| panic!("{label} published metadata JSON: {error}"));
        let object = metadata
            .as_object()
            .unwrap_or_else(|| panic!("{label} published metadata object"));
        let expected_keys = [
            "host_matches_expected",
            "forwarded_present",
            "x_forwarded_for_present",
            "x_forwarded_host_present",
            "x_forwarded_proto_present",
        ]
        .into_iter()
        .map(String::from)
        .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(
            object
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            expected_keys,
            "{label} metadata schema"
        );
        assert_eq!(
            object
                .get("host_matches_expected")
                .and_then(serde_json::Value::as_bool),
            Some(true),
            "{label} request keeps the configured authority"
        );
        for key in [
            "forwarded_present",
            "x_forwarded_for_present",
            "x_forwarded_host_present",
            "x_forwarded_proto_present",
        ] {
            assert_eq!(
                object.get(key).and_then(serde_json::Value::as_bool),
                Some(false),
                "{label} request must not expose caller forwarding header {key}"
            );
        }
    }
}
