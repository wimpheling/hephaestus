use buffa::Message as _;
use buffa_descriptor::{FieldKind, SingularKind};
use buffa_types::google::protobuf::Timestamp;
use rpc_proto::messages::hephaestus::{
    common::v1::Cursor,
    gateway::v1::{
        GatewayServiceLogMetadata, GatewayServiceLogRecord, GatewayServiceLogStream,
        ListGatewayServiceLogsResponse,
    },
};
use std::collections::BTreeSet;

use super::common::{pool, reachable_sensitive_field, sensitive_fields};

#[test]
fn application_payloads_are_typed_and_responses_are_secret_safe() {
    let pool = pool();
    let allowed_bytes = BTreeSet::from([
        "hephaestus.artifact.v1.StreamArtifactResponse.contents",
        "hephaestus.gateway.v1.GatewayServiceLogRecord.contents",
        "hephaestus.identity.v1.CreateBrowserSessionRequest.sid",
        "hephaestus.repository_browser.v1.StreamFileResponse.contents",
        "hephaestus.pat.v1.PersonalAccessTokenValue.value",
        "hephaestus.secret.v1.SecretValue.value",
        "hephaestus.release.v1.CreateUiBrowserHandoffRequest.handoff_secret",
    ]);

    for message in pool
        .messages()
        .iter()
        .filter(|message| message.full_name().starts_with("hephaestus."))
    {
        for field in message.fields() {
            let field_name = format!("{}.{}", message.full_name(), field.name());
            assert!(
                !matches!(field.kind(), FieldKind::Map { .. }),
                "{field_name} is an untyped map escape hatch"
            );
            if matches!(
                field.kind(),
                FieldKind::Singular(SingularKind::Scalar(buffa_descriptor::ScalarType::Bytes))
            ) {
                assert!(
                    allowed_bytes.contains(field_name.as_str()),
                    "{field_name} is an unreviewed opaque byte payload"
                );
            }
            if let FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index)) = field.kind()
            {
                let nested = pool.message(index).full_name();
                assert!(
                    !matches!(
                        nested,
                        "google.protobuf.Struct"
                            | "google.protobuf.Value"
                            | "google.protobuf.ListValue"
                            | "google.protobuf.Any"
                            | "google.protobuf.FieldMask"
                    ),
                    "{field_name} uses forbidden untyped or unjustified WKT {nested}"
                );
            }
        }
    }

    assert_eq!(
        sensitive_fields(&pool),
        BTreeSet::from([
            "hephaestus.identity.v1.CreateBrowserSessionRequest.sid".to_owned(),
            "hephaestus.pat.v1.PersonalAccessTokenValue.value".to_owned(),
            "hephaestus.secret.v1.SecretValue.value".to_owned(),
            "hephaestus.release.v1.CreateUiBrowserHandoffRequest.handoff_secret".to_owned(),
        ])
    );
    for service in pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
    {
        for method in service.methods() {
            let mut visited = BTreeSet::new();
            let qualified = format!("{}/{}", service.full_name(), method.name());
            let found =
                reachable_sensitive_field(&pool, pool.message(method.output()), &mut visited);
            if matches!(
                qualified.as_str(),
                "hephaestus.pat.v1.PersonalAccessTokenService/CreatePersonalAccessToken"
                    | "hephaestus.pat.v1.PersonalAccessTokenService/RotatePersonalAccessToken"
            ) {
                assert_eq!(
                    found.as_deref(),
                    Some("hephaestus.pat.v1.PersonalAccessTokenValue.value"),
                    "{qualified} must expose only its reviewed one-time bearer value"
                );
            } else {
                assert_eq!(found, None, "{qualified} exposes a sensitive field");
            }
        }
    }
}

#[test]
fn service_log_response_worst_case_stays_below_rpc_budget() {
    const MAX_PAGE_CONTENTS: usize = 512 * 1024;
    const MAX_RECORDS: usize = 100;
    const RPC_RESPONSE_LIMIT: usize = 1_048_576;
    let remainder = MAX_PAGE_CONTENTS % MAX_RECORDS;
    let base_record_bytes = MAX_PAGE_CONTENTS / MAX_RECORDS;
    let metadata = GatewayServiceLogMetadata {
        epoch_present: true,
        acknowledged_through: Some(u64::MAX),
        retained_bytes: u64::MAX,
        retained_chunks: u64::MAX,
        producer_dropped_chunks: u64::MAX,
        producer_dropped_bytes: u64::MAX,
        provider_lagged_events: u64::MAX,
        storage_dropped_chunks: u64::MAX,
        storage_dropped_bytes: u64::MAX,
        evicted_chunks: u64::MAX,
        evicted_bytes: u64::MAX,
        earliest_retained_sequence: Some(u64::MAX),
        ..Default::default()
    };
    let records = (0..MAX_RECORDS)
        .map(|index| GatewayServiceLogRecord {
            sequence: u64::MAX - index as u64,
            stream: GatewayServiceLogStream::GATEWAY_SERVICE_LOG_STREAM_STDERR.into(),
            observed_at: Timestamp {
                seconds: i64::MAX,
                nanos: 999_999_999,
                ..Default::default()
            }
            .into(),
            stored_at: Timestamp {
                seconds: i64::MAX,
                nanos: 999_999_999,
                ..Default::default()
            }
            .into(),
            contents: vec![0xa5; base_record_bytes + usize::from(index < remainder)],
            ..Default::default()
        })
        .collect::<Vec<_>>();
    let response = ListGatewayServiceLogsResponse {
        metadata: metadata.into(),
        records,
        history_incomplete: true,
        next_after: Cursor {
            value: "cursor".repeat(32),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };

    let encoded = response.encode_to_vec();
    assert_eq!(
        response
            .records
            .iter()
            .map(|record| record.contents.len())
            .sum::<usize>(),
        MAX_PAGE_CONTENTS
    );
    assert_eq!(encoded.len(), response.encoded_len() as usize);
    assert!(
        encoded.len() < RPC_RESPONSE_LIMIT,
        "worst-case log page encoded to {} bytes",
        encoded.len()
    );
}
