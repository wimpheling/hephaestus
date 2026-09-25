use buffa::ExtensionSet as _;
use buffa_descriptor::{FieldKind, SingularKind};
use rpc_proto::messages::hephaestus::options::v1::SENSITIVE;
use std::collections::BTreeSet;

use super::common::pool;

#[test]
fn product_event_payloads_exclude_sensitive_and_high_volume_shapes() {
    let pool = pool();
    let mut pending = vec![
        pool.message_by_name("hephaestus.event.v1.ProductEvent")
            .expect("canonical product event"),
    ];
    let mut visited = BTreeSet::new();
    let forbidden_names = [
        "plaintext",
        "ciphertext",
        "credential",
        "token",
        "password",
        "environment",
        "parameter",
        "diagnostic",
        "log",
        "metric",
        "contents",
        "payload_json",
    ];
    let forbidden_types = BTreeSet::from([
        "hephaestus.common.v1.Diagnostic",
        "hephaestus.common.v1.RuntimeMetric",
        "hephaestus.secret.v1.SecretValue",
    ]);

    while let Some(message) = pending.pop() {
        if !visited.insert(message.full_name()) {
            continue;
        }
        for field in message.fields() {
            assert!(
                forbidden_names
                    .iter()
                    .all(|forbidden| !field.name().contains(forbidden)),
                "{}.{} has a forbidden product-event shape",
                message.full_name(),
                field.name()
            );
            assert!(
                !field
                    .options()
                    .and_then(|options| options.extension(&SENSITIVE))
                    .unwrap_or(false),
                "{}.{} is sensitive and cannot be an event field",
                message.full_name(),
                field.name()
            );
            if let FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index)) = field.kind()
            {
                let nested = pool.message(index);
                assert!(
                    !forbidden_types.contains(nested.full_name()),
                    "{}.{} embeds forbidden type {}",
                    message.full_name(),
                    field.name(),
                    nested.full_name()
                );
                if nested.full_name().starts_with("hephaestus.event.v1.") {
                    pending.push(nested);
                }
            }
        }
    }
}

#[test]
fn product_event_watches_are_scoped_bounded_and_race_free_by_contract() {
    let pool = pool();
    let service = pool
        .service_by_name("hephaestus.event.v1.ProductEventService")
        .expect("product event service");
    let expected = BTreeSet::from([
        "WatchIdentity",
        "WatchOrganization",
        "WatchProject",
        "WatchRepository",
        "WatchRun",
        "WatchAgentInstance",
    ]);
    assert_eq!(
        service
            .methods()
            .iter()
            .map(buffa_descriptor::MethodDescriptor::name)
            .collect::<BTreeSet<_>>(),
        expected
    );
    assert!(
        service
            .methods()
            .iter()
            .all(buffa_descriptor::MethodDescriptor::is_server_streaming)
    );

    let identity_request = pool
        .message_by_name("hephaestus.event.v1.WatchIdentityRequest")
        .expect("identity watch request");
    assert!(
        identity_request
            .fields()
            .iter()
            .all(|field| !field.name().contains("identity")),
        "identity scope must come from authenticated metadata"
    );
    for method in service.methods() {
        let request = pool.message(method.input());
        for field in ["resume_cursor", "max_events", "max_total_bytes"] {
            assert!(
                request.field_by_name(field).is_some(),
                "{}/{field}",
                method.name()
            );
        }
        let response = pool.message(method.output());
        let item = response
            .oneofs()
            .iter()
            .find(|oneof| oneof.name() == "item")
            .expect("watch item oneof");
        let item_fields = item
            .field_indices()
            .iter()
            .map(|index| response.fields()[usize::from(*index)].name())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            item_fields,
            BTreeSet::from([
                "snapshot_barrier",
                "event",
                "retention_gap",
                "access_revoked"
            ])
        );
    }

    let barrier = pool
        .message_by_name("hephaestus.event.v1.ScopeSnapshotBarrier")
        .expect("race-free snapshot barrier");
    for field in [
        "scope",
        "committed_cursor",
        "aggregate_versions",
        "schema_version",
    ] {
        assert!(barrier.field_by_name(field).is_some());
    }
    let contract = include_str!("../../../../../../ARCHITECTURE.md");
    for required in [
        "establishes the live subscription first",
        "buffers events strictly after the barrier",
        "loads its ordinary typed RPC snapshots",
        "before each delivery",
    ] {
        assert!(
            contract.contains(required),
            "watch handshake omits {required}"
        );
    }
}
