use buffa_descriptor::{DescriptorPool, FieldKind, MessageDescriptor, ScalarType, SingularKind};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

use super::common::{enum_field_is, message_field_is, pool};

#[test]
fn product_event_scope_aggregate_and_change_enums_are_frozen() {
    let pool = pool();
    for (enum_name, expected) in [
        (
            "hephaestus.event.v1.EventScopeKind",
            &[
                "EVENT_SCOPE_KIND_UNSPECIFIED",
                "EVENT_SCOPE_KIND_IDENTITY",
                "EVENT_SCOPE_KIND_ORGANIZATION",
                "EVENT_SCOPE_KIND_PROJECT",
                "EVENT_SCOPE_KIND_REPOSITORY",
                "EVENT_SCOPE_KIND_RUN",
                "EVENT_SCOPE_KIND_AGENT_INSTANCE",
            ][..],
        ),
        (
            "hephaestus.event.v1.AggregateType",
            &[
                "AGGREGATE_TYPE_UNSPECIFIED",
                "AGGREGATE_TYPE_IDENTITY_ORGANIZATIONS",
                "AGGREGATE_TYPE_ORGANIZATION",
                "AGGREGATE_TYPE_PROJECT",
                "AGGREGATE_TYPE_REPOSITORY",
                "AGGREGATE_TYPE_REPOSITORY_REF",
                "AGGREGATE_TYPE_BUILD",
                "AGGREGATE_TYPE_RELEASE",
                "AGGREGATE_TYPE_AGENT_INSTANCE",
                "AGGREGATE_TYPE_RUN",
                "AGGREGATE_TYPE_REVIEW",
                "AGGREGATE_TYPE_SECRET_METADATA",
                "AGGREGATE_TYPE_SECRET_GRANT",
                "AGGREGATE_TYPE_SECRET_IMPORT",
                "AGGREGATE_TYPE_AGENT_SECRET_BINDING",
                "AGGREGATE_TYPE_ARTIFACT",
                "AGGREGATE_TYPE_IDENTITY_PROFILE",
                "AGGREGATE_TYPE_REGISTRY_PUBLICATION",
                "AGGREGATE_TYPE_GATEWAY",
            ][..],
        ),
        (
            "hephaestus.event.v1.ChangeKind",
            &[
                "CHANGE_KIND_UNSPECIFIED",
                "CHANGE_KIND_CREATED",
                "CHANGE_KIND_UPDATED",
                "CHANGE_KIND_STATE_CHANGED",
                "CHANGE_KIND_REMOVED",
            ][..],
        ),
    ] {
        let actual = pool
            .enum_by_name(enum_name)
            .unwrap_or_else(|| panic!("missing {enum_name}"))
            .values()
            .iter()
            .map(buffa_descriptor::EnumValueDescriptor::name)
            .collect::<Vec<_>>();
        assert_eq!(actual, expected, "{enum_name}");
    }
}

#[derive(Deserialize)]
struct ReducerCoverage {
    schema: String,
    variants: Vec<ReducerVariant>,
}

#[derive(Deserialize)]
struct ReducerVariant {
    field: String,
    rust_projection: String,
    phoenix_reducer: String,
}

fn expected_payload_fields() -> BTreeMap<&'static str, BTreeSet<&'static str>> {
    BTreeMap::from([
        (
            "identity_organizations_changed",
            BTreeSet::from(["organization_id", "change", "state"]),
        ),
        ("organization_changed", BTreeSet::from(["change", "state"])),
        (
            "project_changed",
            BTreeSet::from(["organization_id", "change", "state"]),
        ),
        (
            "repository_changed",
            BTreeSet::from(["project_id", "change", "state"]),
        ),
        (
            "repository_ref_changed",
            BTreeSet::from(["change", "state"]),
        ),
        (
            "build_changed",
            BTreeSet::from(["repository_id", "change", "state"]),
        ),
        (
            "release_changed",
            BTreeSet::from(["repository_id", "change", "state"]),
        ),
        (
            "agent_instance_changed",
            BTreeSet::from(["project_id", "change", "state"]),
        ),
        (
            "run_changed",
            BTreeSet::from(["project_id", "repository_id", "change", "state"]),
        ),
        (
            "review_changed",
            BTreeSet::from(["run_id", "change", "state"]),
        ),
        (
            "secret_metadata_changed",
            BTreeSet::from(["owner_id", "change", "state"]),
        ),
        (
            "secret_grant_changed",
            BTreeSet::from(["secret_id", "target_id", "change", "state"]),
        ),
        (
            "secret_import_changed",
            BTreeSet::from(["secret_id", "target_id", "change", "state"]),
        ),
        (
            "agent_secret_binding_changed",
            BTreeSet::from(["agent_instance_id", "secret_import_id", "change", "state"]),
        ),
        (
            "artifact_changed",
            BTreeSet::from(["release_id", "build_id", "change", "state"]),
        ),
        (
            "identity_profile_changed",
            BTreeSet::from(["change", "state"]),
        ),
        (
            "registry_publication_changed",
            BTreeSet::from(["change", "state"]),
        ),
        ("gateway_changed", BTreeSet::from(["change", "state"])),
    ])
}

fn validate_payload_shapes(pool: &DescriptorPool, event: &MessageDescriptor) -> BTreeSet<String> {
    let payload = event
        .oneofs()
        .iter()
        .find(|oneof| oneof.name() == "payload")
        .expect("typed payload oneof");
    assert!(!payload.is_synthetic());
    let expected_payload_fields = expected_payload_fields();
    payload
        .field_indices()
        .iter()
        .map(|field_index| {
            let variant = &event.fields()[usize::from(*field_index)];
            let FieldKind::Singular(SingularKind::Message(message_index)) = variant.kind() else {
                panic!("{} is not a typed payload", variant.name());
            };
            let message = pool.message(message_index);
            let expected = expected_payload_fields
                .get(variant.name())
                .unwrap_or_else(|| panic!("{} has no frozen payload shape", variant.name()));
            assert_eq!(
                message
                    .fields()
                    .iter()
                    .map(buffa_descriptor::FieldDescriptor::name)
                    .collect::<BTreeSet<_>>(),
                *expected,
                "{} payload fields changed",
                variant.name()
            );
            validate_payload_field_types(pool, message);
            variant.name().to_owned()
        })
        .collect()
}

fn validate_payload_field_types(pool: &DescriptorPool, message: &MessageDescriptor) {
    for field in message.fields() {
        let expected_type = match field.name() {
            "change" => "hephaestus.event.v1.ChangeKind",
            "state" => "hephaestus.event.v1.LifecycleState",
            _ => "hephaestus.common.v1.OpaqueId",
        };
        let actual_type = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index)) => pool.message(index).full_name(),
            FieldKind::Singular(SingularKind::Enum(index)) => pool.enumeration(index).full_name(),
            _ => panic!(
                "{}.{} is not singular and typed",
                message.full_name(),
                field.name()
            ),
        };
        assert_eq!(
            actual_type,
            expected_type,
            "{}.{}",
            message.full_name(),
            field.name()
        );
    }
}

#[test]
fn product_events_have_one_canonical_envelope_and_complete_reducer_manifest() {
    let pool = pool();
    let event = pool
        .message_by_name("hephaestus.event.v1.ProductEvent")
        .expect("canonical product event");
    let actual_envelope = event
        .fields()
        .iter()
        .filter(|field| field.oneof_index().is_none())
        .map(buffa_descriptor::FieldDescriptor::name)
        .collect::<BTreeSet<_>>();
    assert_eq!(
        actual_envelope,
        BTreeSet::from([
            "event_id",
            "cursor",
            "scope",
            "aggregate_type",
            "aggregate_id",
            "aggregate_version",
            "occurred_at",
            "provenance",
            "schema_version",
        ])
    );
    for (field, message_type) in [
        ("event_id", "hephaestus.common.v1.OpaqueId"),
        ("cursor", "hephaestus.common.v1.Cursor"),
        ("scope", "hephaestus.event.v1.EventScope"),
        ("aggregate_id", "hephaestus.common.v1.OpaqueId"),
        ("occurred_at", "google.protobuf.Timestamp"),
        ("provenance", "hephaestus.event.v1.EventProvenance"),
    ] {
        assert!(message_field_is(&pool, event, field, message_type));
    }
    assert!(enum_field_is(
        &pool,
        event,
        "aggregate_type",
        "hephaestus.event.v1.AggregateType"
    ));
    for (field, scalar) in [
        ("aggregate_version", ScalarType::Uint64),
        ("schema_version", ScalarType::Uint32),
    ] {
        assert!(event.field_by_name(field).is_some_and(|descriptor| {
            matches!(
                descriptor.kind(),
                FieldKind::Singular(SingularKind::Scalar(actual)) if actual == scalar
            )
        }));
    }

    let actual_variants = validate_payload_shapes(&pool, event);

    let coverage: ReducerCoverage = toml::from_str(include_str!(
        "../../../../../../proto/event-reducer-coverage.toml"
    ))
    .expect("event reducer coverage manifest");
    assert_eq!(coverage.schema, event.full_name());
    let manifested = coverage
        .variants
        .iter()
        .map(|variant| {
            assert!(!variant.rust_projection.trim().is_empty());
            assert!(!variant.phoenix_reducer.trim().is_empty());
            variant.field.clone()
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(actual_variants, manifested);
    assert_eq!(coverage.variants.len(), 18);
}
