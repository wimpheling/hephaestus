use super::{GatewayServiceLogRpcFixture, opaque_id, service_log_rpc_token};
use connectrpc::{
    Protocol,
    client::{CallOptions, ClientConfig, Http2Connection},
    error::ErrorCode,
};
use identity_domain::BrowserSessionSid;
use rpc_proto::connect::hephaestus::gateway::v1::GatewayServiceClient;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    gateway::v1::{
        GatewayServiceLogScope, GatewayServiceLogStream, GetProjectServiceLogMetadataRequest,
        ListGatewayServiceLogsRequest,
    },
};

// Keep metadata, paging, authorization, and revocation assertions in one
// ordered RPC proof so their shared transport state remains explicit.
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
    owner_browser_session: BrowserSessionSid,
    outsider_id: uuid::Uuid,
    outsider_browser_session: BrowserSessionSid,
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
        member_browser_session,
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
    let owner_token =
        service_log_rpc_token(&owner_id, owner_browser_session, "ListGatewayServiceLogs");
    let metadata_token = service_log_rpc_token(
        &owner_id,
        owner_browser_session,
        "GetProjectServiceLogMetadata",
    );
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
    let wrong_audience_token =
        service_log_rpc_token(&owner_id, owner_browser_session, "ListGatewayServiceLogs");
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

    let outsider_token = service_log_rpc_token(
        &outsider_id,
        outsider_browser_session,
        "ListGatewayServiceLogs",
    );
    let error = client
        .list_gateway_service_logs_with_options(
            request(scope.clone(), None),
            CallOptions::default().with_header("authorization", format!("Bearer {outsider_token}")),
        )
        .await
        .expect_err("outsider service log RPC must fail");
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("rpc-service-log"));

    let member_token =
        service_log_rpc_token(&member_id, member_browser_session, "ListGatewayServiceLogs");
    let member_metadata_token = service_log_rpc_token(
        &member_id,
        member_browser_session,
        "GetProjectServiceLogMetadata",
    );
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
