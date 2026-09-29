use super::{
    GUEST_SERVICE_LOG_STDERR_MARKER, GUEST_SERVICE_LOG_STDOUT_MARKER, GatewayServiceGuestLogProof,
    opaque_id, service_log_rpc_token,
};
use connectrpc::{
    Protocol,
    client::{CallOptions, ClientConfig, Http2Connection},
};
use gateway_edge::MAX_SERVICE_LOG_INSTANCE_CHUNKS;
use identity_domain::BrowserSessionSid;
use rpc_proto::connect::hephaestus::gateway::v1::GatewayServiceClient;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    gateway::v1::{GatewayServiceLogScope, GatewayServiceLogStream, ListGatewayServiceLogsRequest},
};
use std::time::{Duration, Instant};

// Keep the public endpoint, authenticated streams, and ordered marker checks in
// one helper so this end-to-end proof remains easy to audit.
#[cfg(feature = "test-fixtures")]
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
/// Invokes the public endpoint and reads both authenticated log streams.
///
/// # Panics
///
/// Panics when the endpoint, transport, authentication, pagination, metadata,
/// or marker assertions fail.
pub async fn exercise_gateway_service_guest_log(
    http_addr: std::net::SocketAddr,
    gateway_id: uuid::Uuid,
    revision_id: uuid::Uuid,
    instance_id: uuid::Uuid,
    project_id: uuid::Uuid,
    fencing_token: i64,
    owner_id: uuid::Uuid,
    owner_browser_session: BrowserSessionSid,
    public_url: &str,
) -> GatewayServiceGuestLogProof {
    assert!(fencing_token > 0);

    let response = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("guest service-log HTTP client")
        .get(format!("{public_url}/gateway/service/log"))
        .send()
        .await
        .expect("invoke public guest service-log endpoint");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .expect("guest service-log content type");
    assert!(
        content_type.starts_with("text/plain"),
        "guest service-log endpoint must return text/plain: {content_type}"
    );
    let body = response
        .bytes()
        .await
        .expect("read guest service-log endpoint body");
    assert_eq!(body.as_ref(), b"log-emitted");

    let scope = GatewayServiceLogScope {
        project_id: opaque_id(project_id).into(),
        gateway_id: opaque_id(gateway_id).into(),
        revision_id: opaque_id(revision_id).into(),
        instance_id: opaque_id(instance_id).into(),
        fencing_token: u64::try_from(fencing_token).expect("positive fencing token"),
        ..Default::default()
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    let uri: axum::http::Uri = format!("http://{http_addr}")
        .parse()
        .expect("guest service-log RPC URI");
    let connection = tokio::time::timeout(
        deadline.saturating_duration_since(Instant::now()),
        Http2Connection::connect_plaintext(uri.clone()),
    )
    .await
    .expect("guest service-log RPC connection deadline")
    .expect("guest service-log RPC connection")
    .shared(4);
    let client = GatewayServiceClient::new(
        connection,
        ClientConfig::new(uri).with_protocol(Protocol::Connect),
    );

    let max_epoch_records = usize::try_from(MAX_SERVICE_LOG_INSTANCE_CHUNKS)
        .expect("service-log epoch chunk quota fits usize");
    loop {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for guest service-log markers"
        );
        let mut after = None;
        let mut records = Vec::new();
        let mut metadata = None;
        let mut history_incomplete = false;
        let mut page_count = 0_usize;
        loop {
            page_count = page_count
                .checked_add(1)
                .expect("guest service-log page count overflow");
            assert!(
                page_count <= max_epoch_records,
                "guest service-log pagination exceeded the production epoch chunk quota"
            );
            let request = ListGatewayServiceLogsRequest {
                scope: scope.clone().into(),
                limit: 1,
                after: after
                    .clone()
                    .map(|value| Cursor {
                        value,
                        ..Default::default()
                    })
                    .into(),
                ..Default::default()
            };
            let token =
                service_log_rpc_token(&owner_id, owner_browser_session, "ListGatewayServiceLogs");
            let remaining = deadline.saturating_duration_since(Instant::now());
            let response = tokio::time::timeout(
                remaining,
                client.list_gateway_service_logs_with_options(
                    request,
                    CallOptions::default().with_header("authorization", format!("Bearer {token}")),
                ),
            )
            .await
            .expect("guest service-log RPC deadline")
            .expect("authorized guest service-log RPC page")
            .into_owned();
            if metadata.is_none() {
                metadata = response.metadata.as_option().cloned();
            }
            history_incomplete |= response.history_incomplete;
            records.extend(response.records);
            assert!(
                records.len() <= max_epoch_records,
                "guest service-log records exceeded the production epoch chunk quota"
            );
            let next_after = response
                .next_after
                .as_option()
                .map(|cursor| cursor.value.clone());
            assert!(
                !(after.is_some() && next_after == after),
                "guest service-log RPC cursor did not advance"
            );
            after = next_after;
            if after.is_none() {
                break;
            }
        }

        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let mut previous_sequence = None;
        for record in &records {
            if let Some(previous_sequence) = previous_sequence {
                assert!(
                    record.sequence > previous_sequence,
                    "guest service-log sequences must increase: {previous_sequence} then {}",
                    record.sequence
                );
            }
            previous_sequence = Some(record.sequence);
            if record.stream == GatewayServiceLogStream::Stdout {
                stdout.extend_from_slice(&record.contents);
            } else if record.stream == GatewayServiceLogStream::Stderr {
                stderr.extend_from_slice(&record.contents);
            } else {
                panic!("guest service-log RPC returned an unexpected stream");
            }
        }

        if metadata.as_ref().is_some_and(|metadata| {
            metadata.epoch_present
                && metadata.acknowledged_through.is_some()
                && !history_incomplete
                && marker_count(&stdout, GUEST_SERVICE_LOG_STDOUT_MARKER) == 1
                && marker_count(&stderr, GUEST_SERVICE_LOG_STDERR_MARKER) == 1
        }) {
            assert!(
                !records.is_empty(),
                "guest service-log RPC returned no records"
            );
            return GatewayServiceGuestLogProof {
                project_id,
                gateway_id,
                revision_id,
                instance_id,
                fencing_token,
                stdout,
                stderr,
            };
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for guest service-log markers"
        );
        let remaining = deadline.saturating_duration_since(Instant::now());
        tokio::time::sleep(Duration::from_millis(100).min(remaining)).await;
    }
}

#[cfg(feature = "test-fixtures")]
/// Counts non-overlapping occurrences of a marker in a byte stream.
pub fn marker_count(bytes: &[u8], marker: &[u8]) -> usize {
    bytes
        .windows(marker.len())
        .filter(|window| *window == marker)
        .count()
}
