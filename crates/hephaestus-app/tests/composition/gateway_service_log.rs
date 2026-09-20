//! Test composition boundary for authenticated gateway service-log RPC probes.
//!
//! Generated Connect clients and protobuf messages are deliberately contained here.
//! Callers receive only plain IDs, bytes, and test-owned proof data.
use connectrpc::{
    Protocol,
    client::{CallOptions, ClientConfig, Http2Connection},
    error::ErrorCode,
};
use gateway_edge::MAX_SERVICE_LOG_INSTANCE_CHUNKS;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use rpc_proto::{
    connect::hephaestus::gateway::v1::GatewayServiceClient,
    messages::hephaestus::{
        common::v1::{Cursor, OpaqueId},
        gateway::v1::{
            GatewayServiceLogScope, GatewayServiceLogStream, GetProjectServiceLogMetadataRequest,
            ListGatewayServiceLogsRequest,
        },
    },
};
use std::time::{Duration, Instant};
use time::OffsetDateTime;

#[cfg(feature = "test-fixtures")]
#[derive(Debug)]
/// Proof returned after the public service-log request and authenticated read.
pub struct GatewayServiceGuestLogProof {
    /// Project scope used for the retained records.
    pub(super) project_id: uuid::Uuid,
    /// Gateway scope used for the retained records.
    pub(super) gateway_id: uuid::Uuid,
    /// Immutable service revision scope used for the retained records.
    pub(super) revision_id: uuid::Uuid,
    /// Service instance that emitted the records.
    pub(super) instance_id: uuid::Uuid,
    /// Fencing epoch associated with the records.
    pub(super) fencing_token: i64,
    /// Concatenated stdout bytes observed before shutdown.
    pub(super) stdout: Vec<u8>,
    /// Concatenated stderr bytes observed before shutdown.
    pub(super) stderr: Vec<u8>,
}

#[cfg(feature = "test-fixtures")]
/// Ordinary stdout marker emitted by the cooking fixture.
pub const GUEST_SERVICE_LOG_STDOUT_MARKER: &[u8] = b"service-log-stdout=ordinary\n";
#[cfg(feature = "test-fixtures")]
/// Ordinary stderr marker emitted by the cooking fixture.
pub const GUEST_SERVICE_LOG_STDERR_MARKER: &[u8] = b"service-log-stderr=ordinary\n";

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
            let token = service_log_rpc_token(&owner_id, "ListGatewayServiceLogs");
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

#[cfg(feature = "test-fixtures")]
#[derive(Debug)]
/// Plain test data prepared by the golden SQL fixture before transport checks.
pub struct GatewayServiceLogRpcFixture {
    /// Gateway scope used by the RPC request.
    pub(super) gateway_id: uuid::Uuid,
    /// Immutable revision scope used by the RPC request.
    pub(super) revision_id: uuid::Uuid,
    /// Service instance scope used by the RPC request.
    pub(super) instance_id: uuid::Uuid,
    /// Project scope used by the RPC request.
    pub(super) project_id: uuid::Uuid,
    /// Fencing epoch used by the RPC request.
    pub(super) fencing_token: i64,
    /// First sequence expected after the seeded records.
    pub(super) first_sequence: i64,
    /// Epoch acknowledgement watermark before the seeded records.
    pub(super) baseline_acknowledged: i64,
    /// Retained bytes before the seeded records.
    pub(super) baseline_bytes: i64,
    /// Retained chunks before the seeded records.
    pub(super) baseline_chunks: i64,
    /// Seeded payloads returned by the paged RPC.
    pub(super) payloads: [Vec<u8>; 2],
    /// Total bytes in the seeded payloads.
    pub(super) payload_bytes: i64,
    /// Foreign project used by the authorization denial assertion.
    pub(super) foreign_project: uuid::Uuid,
    /// Member whose access is revoked between transport assertions.
    pub(super) member_id: uuid::Uuid,
}

#[allow(
    clippy::cognitive_complexity,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
/// Exercises authenticated metadata and log paging at the RPC boundary.
///
/// # Panics
///
/// Panics when transport, authentication, paging, metadata, or authorization
/// assertions fail.
pub async fn exercise_gateway_service_log_rpc<F, Fut>(
    running: &hephaestus_app::RunningHephaestus,
    fixture: GatewayServiceLogRpcFixture,
    owner_id: uuid::Uuid,
    outsider_id: uuid::Uuid,
    revoke_member: F,
) where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let GatewayServiceLogRpcFixture {
        gateway_id,
        revision_id,
        instance_id,
        project_id,
        fencing_token,
        first_sequence,
        baseline_acknowledged,
        baseline_bytes,
        baseline_chunks,
        payloads,
        payload_bytes,
        foreign_project,
        member_id,
    } = fixture;
    let scope = GatewayServiceLogScope {
        project_id: opaque_id(project_id).into(),
        gateway_id: opaque_id(gateway_id).into(),
        revision_id: opaque_id(revision_id).into(),
        instance_id: opaque_id(instance_id).into(),
        fencing_token: u64::try_from(fencing_token).expect("positive fencing token"),
        ..Default::default()
    };
    let uri: axum::http::Uri = format!("http://{}", running.http_addr())
        .parse()
        .expect("gateway log RPC URI");
    let connection = Http2Connection::connect_plaintext(uri.clone())
        .await
        .expect("gateway log RPC connection")
        .shared(4);
    let client = GatewayServiceClient::new(
        connection,
        ClientConfig::new(uri).with_protocol(Protocol::Connect),
    );
    let metadata_request = |project_id: uuid::Uuid| GetProjectServiceLogMetadataRequest {
        project_id: opaque_id(project_id).into(),
        ..Default::default()
    };
    let owner_token = service_log_rpc_token(&owner_id, "ListGatewayServiceLogs");
    let metadata_token = service_log_rpc_token(&owner_id, "GetProjectServiceLogMetadata");
    let metadata = client
        .get_project_service_log_metadata_with_options(
            metadata_request(project_id),
            CallOptions::default().with_header("authorization", format!("Bearer {metadata_token}")),
        )
        .await
        .expect("authorized project service-log metadata")
        .into_owned();
    let metadata = metadata
        .metadata
        .as_option()
        .expect("project service-log metadata response");
    assert!(metadata.usage_present);
    assert_eq!(metadata.storage_dropped_chunks, 7);
    assert_eq!(metadata.storage_dropped_bytes, 123);

    let error = client
        .get_project_service_log_metadata_with_options(
            metadata_request(foreign_project),
            CallOptions::default().with_header("authorization", format!("Bearer {metadata_token}")),
        )
        .await
        .expect_err("cross-project metadata must fail");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("999"));

    let error = client
        .get_project_service_log_metadata_with_options(
            metadata_request(project_id),
            CallOptions::default(),
        )
        .await
        .expect_err("missing metadata authorization must fail");
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    let wrong_audience_token = service_log_rpc_token(&owner_id, "ListGatewayServiceLogs");
    let error = client
        .get_project_service_log_metadata_with_options(
            metadata_request(project_id),
            CallOptions::default()
                .with_header("authorization", format!("Bearer {wrong_audience_token}")),
        )
        .await
        .expect_err("wrong metadata audience must fail");
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    let error = client
        .get_project_service_log_metadata_with_options(
            GetProjectServiceLogMetadataRequest::default(),
            CallOptions::default().with_header("authorization", format!("Bearer {metadata_token}")),
        )
        .await
        .expect_err("missing project id must fail");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    let error = client
        .get_project_service_log_metadata_with_options(
            metadata_request(uuid::Uuid::nil()),
            CallOptions::default().with_header("authorization", format!("Bearer {metadata_token}")),
        )
        .await
        .expect_err("nil project id must fail");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    let request =
        |scope: GatewayServiceLogScope, after: Option<String>| ListGatewayServiceLogsRequest {
            scope: scope.into(),
            limit: 1,
            after: after
                .map(|value| Cursor {
                    value,
                    ..Default::default()
                })
                .into(),
            ..Default::default()
        };
    let first = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), None),
            CallOptions::default().with_header("authorization", format!("Bearer {owner_token}")),
        )
        .await
        .expect("authorized service log RPC page")
        .into_owned();
    assert_eq!(first.records.len(), 1);
    assert_eq!(first.records[0].contents, payloads[0]);
    assert_eq!(first.records[0].stream, GatewayServiceLogStream::Stdout);
    let first_metadata = first
        .metadata
        .as_option()
        .expect("service log RPC metadata");
    assert!(first_metadata.epoch_present);
    assert_eq!(
        first_metadata.acknowledged_through,
        Some(u64::try_from(baseline_acknowledged.max(first_sequence + 1)).expect("ack watermark"))
    );
    assert_eq!(
        first_metadata.retained_bytes,
        u64::try_from(baseline_bytes + payload_bytes).expect("retained bytes")
    );
    assert_eq!(
        first_metadata.retained_chunks,
        u64::try_from(baseline_chunks + 2).expect("retained chunks")
    );
    let cursor = first
        .next_after
        .as_option()
        .expect("first service log page cursor")
        .value
        .clone();
    let second = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), Some(cursor.clone())),
            CallOptions::default().with_header("authorization", format!("Bearer {owner_token}")),
        )
        .await
        .expect("authorized service log RPC continuation")
        .into_owned();
    assert_eq!(second.records.len(), 1);
    assert_eq!(second.records[0].contents, payloads[1]);
    assert_eq!(second.records[0].stream, GatewayServiceLogStream::Stdout);
    let second_metadata = second
        .metadata
        .as_option()
        .expect("service log RPC continuation metadata");
    assert_eq!(
        second_metadata.acknowledged_through,
        first_metadata.acknowledged_through
    );
    assert_eq!(
        second_metadata.retained_bytes,
        first_metadata.retained_bytes
    );
    assert_eq!(
        second_metadata.retained_chunks,
        first_metadata.retained_chunks
    );
    assert!(second.next_after.as_option().is_none());

    let mut tampered = cursor.clone().into_bytes();
    let last = tampered.last_mut().expect("nonempty signed cursor");
    *last = if *last == b'A' { b'B' } else { b'A' };
    let tampered = String::from_utf8(tampered).expect("ASCII signed cursor");
    let error = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), Some(tampered)),
            CallOptions::default().with_header("authorization", format!("Bearer {owner_token}")),
        )
        .await
        .expect_err("tampered service log cursor must fail");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    let wrong_scope = GatewayServiceLogScope {
        gateway_id: opaque_id(uuid::Uuid::new_v4()).into(),
        ..scope.clone()
    };
    let error = client
        .list_gateway_service_logs_with_options(
            request(wrong_scope, Some(cursor)),
            CallOptions::default().with_header("authorization", format!("Bearer {owner_token}")),
        )
        .await
        .expect_err("cross-scope service log cursor must fail");
    assert_eq!(error.code, ErrorCode::InvalidArgument);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    let outsider_token = service_log_rpc_token(&outsider_id, "ListGatewayServiceLogs");
    let error = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), None),
            CallOptions::default().with_header("authorization", format!("Bearer {outsider_token}")),
        )
        .await
        .expect_err("outsider service log RPC must fail");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    let member_token = service_log_rpc_token(&member_id, "ListGatewayServiceLogs");
    let member_metadata_token = service_log_rpc_token(&member_id, "GetProjectServiceLogMetadata");
    let member_metadata = client
        .get_project_service_log_metadata_with_options(
            metadata_request(project_id),
            CallOptions::default()
                .with_header("authorization", format!("Bearer {member_metadata_token}")),
        )
        .await
        .expect("current member can read project service-log metadata")
        .into_owned();
    let member_metadata = member_metadata
        .metadata
        .as_option()
        .expect("member project service-log metadata response");
    assert!(member_metadata.usage_present);
    assert_eq!(member_metadata.storage_dropped_chunks, 7);
    assert_eq!(member_metadata.storage_dropped_bytes, 123);
    client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), None),
            CallOptions::default().with_header("authorization", format!("Bearer {member_token}")),
        )
        .await
        .expect("current member can read service logs");
    revoke_member().await;
    let error = client
        .get_project_service_log_metadata_with_options(
            metadata_request(project_id),
            CallOptions::default()
                .with_header("authorization", format!("Bearer {member_metadata_token}")),
        )
        .await
        .expect_err("revoked member project metadata must fail");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("rpc-service-log"));
    let error = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), None),
            CallOptions::default().with_header("authorization", format!("Bearer {member_token}")),
        )
        .await
        .expect_err("revoked service log member must fail");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    let error = client
        .list_gateway_service_logs(request(scope, None))
        .await
        .expect_err("unauthenticated service log RPC must fail");
    assert_eq!(error.code, ErrorCode::Unauthenticated);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    println!(
        "REAL_GATEWAY_SERVICE_LOG_RPC=1 app_role=hephaestus_app payload_cursor=1 denied=outsider+revoked+unauthenticated"
    );
}

#[cfg(feature = "test-fixtures")]
fn opaque_id(value: uuid::Uuid) -> OpaqueId {
    OpaqueId {
        value: value.to_string(),
        ..Default::default()
    }
}

#[cfg(feature = "test-fixtures")]
fn service_log_rpc_token(actor: &uuid::Uuid, method: &str) -> String {
    let now = OffsetDateTime::now_utc().unix_timestamp();
    encode(
        &Header::new(Algorithm::HS256),
        &serde_json::json!({
            "iss": "hephaestus-web-mediator",
            "sub": actor.to_string(),
            "aud": format!("/hephaestus.gateway.v1.GatewayService/{method}"),
            "iat": now,
            "nbf": now,
            "exp": now + 25,
            "jti": uuid::Uuid::new_v4().to_string()
        }),
        &EncodingKey::from_secret(&hephaestus_app::rpc::mediator_signing_key(
            b"golden-internal-command-token-with-sufficient-entropy",
        )),
    )
    .expect("sign service log RPC mediator token")
}
