use super::*;

/// Runs the published service's bounded guest isolation diagnostic through the
/// real Caddy public listener, then proves the public listener cannot serve the
/// separate Caddy administration API, including with a forged admin Host.
pub async fn exercise_published_cooking_service_isolation(public_url: &str, admin_url: &str) {
    let public_port = published_public_loopback_port(public_url, "public Caddy URL");
    let admin_port = loopback_port(admin_url, "admin Caddy URL");
    assert_ne!(public_port, admin_port);
    assert_ne!(public_port, 8080);
    assert_ne!(admin_port, 8080);

    let client = published_cooking_service_client();
    let diagnostic = client
        .get(format!(
            "{public_url}/gateway/service/isolation?admin_port={admin_port}&public_port={public_port}"
        ))
        .send()
        .await
        .expect("published cooking-service isolation request")
        .error_for_status()
        .expect("published cooking-service isolation request succeeds")
        .bytes()
        .await
        .expect("read published cooking-service isolation response");
    let diagnostic: serde_json::Value =
        serde_json::from_slice(&diagnostic).expect("published cooking-service isolation JSON");
    let object = diagnostic
        .as_object()
        .expect("published isolation response object");
    let expected_keys = [
        "schema_version",
        "own_loopback_ok",
        "admin_loopback_blocked",
        "public_loopback_blocked",
        "metadata_blocked",
        "test_net_blocked",
        "runtime_authority_env_absent",
        "runtime_authority_path_absent",
        "broker_socket_absent",
        "secret_mount_absent",
        "control_surface_ok",
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
        "published isolation response must have exactly the reviewed schema"
    );
    assert_eq!(
        object
            .get("schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(1)
    );
    for key in [
        "own_loopback_ok",
        "admin_loopback_blocked",
        "public_loopback_blocked",
        "metadata_blocked",
        "test_net_blocked",
        "runtime_authority_env_absent",
        "runtime_authority_path_absent",
        "broker_socket_absent",
        "secret_mount_absent",
        "control_surface_ok",
    ] {
        assert_eq!(
            object.get(key).and_then(serde_json::Value::as_bool),
            Some(true),
            "published isolation field {key}"
        );
    }

    for host in [None, Some(format!("127.0.0.1:{admin_port}"))] {
        let mut request = client.get(format!("{public_url}/config/"));
        if let Some(host) = host {
            request = request.header(reqwest::header::HOST, host);
        }
        let response = request
            .send()
            .await
            .expect("public Caddy internal-config probe");
        assert_eq!(
            response.status(),
            reqwest::StatusCode::NOT_FOUND,
            "public Caddy listener must deny /config/"
        );
        response
            .bytes()
            .await
            .expect("read public Caddy internal-config denial");
    }
}

pub fn published_public_loopback_port(url: &str, label: &str) -> u16 {
    let url =
        reqwest::Url::parse(url).unwrap_or_else(|error| panic!("{label} is invalid: {error}"));
    let expected_scheme = if env::var("HEPHAESTUS_CADDY_TEST_TLS").as_deref() == Ok("1") {
        "https"
    } else {
        "http"
    };
    assert_eq!(
        url.scheme(),
        expected_scheme,
        "{label} must use the configured disposable harness scheme"
    );
    loopback_port_from_url(&url, label)
}

pub fn loopback_port(url: &str, label: &str) -> u16 {
    let url =
        reqwest::Url::parse(url).unwrap_or_else(|error| panic!("{label} is invalid: {error}"));
    assert_eq!(url.scheme(), "http", "{label} must use HTTP administration");
    loopback_port_from_url(&url, label)
}

pub fn loopback_port_from_url(url: &reqwest::Url, label: &str) -> u16 {
    assert_eq!(
        url.host_str(),
        Some("127.0.0.1"),
        "{label} must use the joined loopback listener"
    );
    url.port()
        .unwrap_or_else(|| panic!("{label} has no explicit port"))
}

#[cfg(feature = "test-fixtures")]
pub async fn assert_guest_gateway_service_log_retained(
    pool: &sqlx::PgPool,
    proof: &GatewayServiceGuestLogProof,
) {
    let epoch: (i64, i64, i64) = sqlx::query_as(
        "SELECT fencing_token, acknowledged_through, retained_chunks
           FROM gateway_service_log_epochs
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5",
    )
    .bind(proof.instance_id)
    .bind(proof.gateway_id)
    .bind(proof.revision_id)
    .bind(proof.project_id)
    .bind(proof.fencing_token)
    .fetch_one(pool)
    .await
    .expect("read retained guest service-log epoch");
    assert_eq!(epoch.0, proof.fencing_token);
    assert!(epoch.1 >= 0);

    let rows: Vec<(i64, String, Vec<u8>)> = sqlx::query_as(
        "SELECT sequence, stream, bytes
           FROM gateway_service_log_chunks
          WHERE instance_id = $1 AND gateway_id = $2 AND revision_id = $3
            AND project_id = $4 AND fencing_token = $5
          ORDER BY sequence",
    )
    .bind(proof.instance_id)
    .bind(proof.gateway_id)
    .bind(proof.revision_id)
    .bind(proof.project_id)
    .bind(proof.fencing_token)
    .fetch_all(pool)
    .await
    .expect("read retained guest service-log chunks");
    assert!(
        !rows.is_empty(),
        "guest service-log chunks were not retained"
    );
    assert_eq!(
        usize::try_from(epoch.2).expect("retained chunk count fits usize"),
        rows.len()
    );

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut previous_sequence = None;
    for (sequence, stream, bytes) in rows {
        if let Some(previous_sequence) = previous_sequence {
            assert!(sequence > previous_sequence);
        }
        previous_sequence = Some(sequence);
        match stream.as_str() {
            "stdout" => stdout.extend_from_slice(&bytes),
            "stderr" => stderr.extend_from_slice(&bytes),
            other => panic!("unexpected retained guest service-log stream: {other}"),
        }
    }
    assert!(
        stdout.starts_with(&proof.stdout),
        "pre-shutdown stdout must remain retained in stream order"
    );
    assert!(
        stderr.starts_with(&proof.stderr),
        "pre-shutdown stderr must remain retained in stream order"
    );
    assert_eq!(
        marker_count(&stdout, GUEST_SERVICE_LOG_STDOUT_MARKER),
        1,
        "retained stdout marker must remain unique"
    );
    assert_eq!(
        marker_count(&stderr, GUEST_SERVICE_LOG_STDERR_MARKER),
        1,
        "retained stderr marker must remain unique"
    );
}
