//! Descriptor-level policy checks for the shared application protocol.

use buffa::ExtensionSet as _;
use buffa_descriptor::{DescriptorPool, FieldKind, MessageDescriptor, ScalarType, SingularKind};
use rpc_proto::messages::hephaestus::options::v1::{
    AUTHORIZATION, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, OPERATION_KIND, SENSITIVE,
};
use serde::Deserialize;
use std::{collections::BTreeMap, collections::BTreeSet, sync::Arc};

const QUERY: i32 = 1;
const MUTATION: i32 = 2;
const SERVER_STREAM: i32 = 3;
const MEDIATOR_JWT: i32 = 1;
const OIDC_BOOTSTRAP: i32 = 2;

fn pool() -> Arc<DescriptorPool> {
    Arc::new(rpc_proto::descriptor_pool().expect("checked-in descriptor set must decode"))
}

#[test]
fn reflection_inventory_contains_every_application_service_and_method() {
    let pool = pool();
    let expected = BTreeSet::from([
        "hephaestus.artifact.v1.ArtifactService",
        "hephaestus.build.v1.BuildService",
        "hephaestus.event.v1.ProductEventService",
        "hephaestus.identity.v1.IdentityService",
        "hephaestus.instance.v1.AgentInstanceService",
        "hephaestus.organization.v1.OrganizationService",
        "hephaestus.project.v1.ProjectService",
        "hephaestus.release.v1.ReleaseService",
        "hephaestus.repository.v1.RepositoryService",
        "hephaestus.repository_browser.v1.RepositoryBrowserService",
        "hephaestus.run.v1.RunService",
        "hephaestus.secret.v1.SecretService",
    ]);
    let application_services = pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
        .collect::<Vec<_>>();
    let actual = application_services
        .iter()
        .map(|service| service.full_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(
        application_services
            .iter()
            .map(|service| service.methods().len())
            .sum::<usize>(),
        51
    );

    let reflector = connectrpc_reflection::Reflector::from_descriptor_pool(pool)
        .expect("descriptor pool must build a tooling reflection index");
    let service_names = reflector.service_names();
    let reflected = service_names
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    assert!(expected.is_subset(&reflected));
    assert!(reflected.contains(connectrpc_reflection::SERVER_REFLECTION_SERVICE_NAME));
    assert!(reflected.contains(connectrpc_reflection::SERVER_REFLECTION_V1ALPHA_SERVICE_NAME));
}

fn message_field_is(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    field_name: &str,
    expected_type: &str,
) -> bool {
    message.field_by_name(field_name).is_some_and(|field| {
        matches!(
            field.kind(),
            FieldKind::Singular(SingularKind::Message(index))
                if pool.message(index).full_name() == expected_type
        )
    })
}

fn enum_field_is(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    field_name: &str,
    expected_type: &str,
) -> bool {
    message.field_by_name(field_name).is_some_and(|field| {
        matches!(
            field.kind(),
            FieldKind::Singular(SingularKind::Enum(index))
                if pool.enumeration(index).full_name() == expected_type
        )
    })
}

fn sensitive_fields(pool: &DescriptorPool) -> BTreeSet<String> {
    pool.messages()
        .iter()
        .filter(|message| message.full_name().starts_with("hephaestus."))
        .flat_map(|message| {
            message.fields().iter().filter_map(move |field| {
                field
                    .options()
                    .and_then(|options| options.extension(&SENSITIVE))
                    .filter(|sensitive| *sensitive)
                    .map(|_| format!("{}.{}", message.full_name(), field.name()))
            })
        })
        .collect()
}

fn reachable_sensitive_field(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    visited: &mut BTreeSet<String>,
) -> Option<String> {
    if !visited.insert(message.full_name().to_owned()) {
        return None;
    }
    for field in message.fields() {
        if field
            .options()
            .and_then(|options| options.extension(&SENSITIVE))
            .unwrap_or(false)
        {
            return Some(format!("{}.{}", message.full_name(), field.name()));
        }
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        if let Some(found) =
            nested.and_then(|index| reachable_sensitive_field(pool, pool.message(index), visited))
        {
            return Some(found);
        }
    }
    None
}

fn message_reaches_named_message(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    target: &str,
    visited: &mut BTreeSet<String>,
) -> bool {
    if message.full_name() == target {
        return true;
    }
    if !visited.insert(message.full_name().to_owned()) {
        return false;
    }
    message.fields().iter().any(|field| {
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        nested.is_some_and(|index| {
            message_reaches_named_message(pool, pool.message(index), target, visited)
        })
    })
}

fn reachable_actor_field(
    pool: &DescriptorPool,
    message: &MessageDescriptor,
    visited: &mut BTreeSet<String>,
) -> Option<String> {
    if !visited.insert(message.full_name().to_owned()) {
        return None;
    }
    for field in message.fields() {
        if field.name().contains("actor") {
            return Some(format!("{}.{}", message.full_name(), field.name()));
        }
        let nested = match field.kind() {
            FieldKind::Singular(SingularKind::Message(index))
            | FieldKind::List(SingularKind::Message(index))
            | FieldKind::Map {
                value: SingularKind::Message(index),
                ..
            } => Some(index),
            _ => None,
        };
        if let Some(found) =
            nested.and_then(|index| reachable_actor_field(pool, pool.message(index), visited))
        {
            return Some(found);
        }
    }
    None
}

fn contains_forbidden_sensitive_name(name: &str) -> bool {
    [
        "plaintext",
        "ciphertext",
        "credential",
        "password",
        "private_key",
        "api_key",
        "access_token",
        "refresh_token",
    ]
    .iter()
    .any(|forbidden| name.contains(forbidden))
}

fn validate_mutation_method(
    pool: &DescriptorPool,
    method: &buffa_descriptor::MethodDescriptor,
    qualified: &str,
) {
    let request = pool.message(method.input());
    assert!(
        message_field_is(
            pool,
            request,
            "context",
            "hephaestus.common.v1.RequestContext"
        ),
        "{qualified} mutation is missing RequestContext"
    );
    let response = pool.message(method.output());
    assert!(
        message_field_is(
            pool,
            response,
            "receipt",
            "hephaestus.common.v1.MutationReceipt"
        ),
        "{qualified} mutation is missing its read-your-writes receipt"
    );
}

fn validate_server_stream_method(
    pool: &DescriptorPool,
    method: &buffa_descriptor::MethodDescriptor,
    qualified: &str,
) {
    assert!(method.is_server_streaming(), "{qualified} is not a stream");
    let watch = method.name().starts_with("Watch");
    let request = pool.message(method.input());
    let required_request_fields: &[&str] = if watch {
        &["resume_cursor", "max_events", "max_total_bytes"]
    } else {
        &["resume_cursor", "max_total_bytes", "max_chunk_bytes"]
    };
    for field in required_request_fields {
        assert!(
            request.field_by_name(field).is_some(),
            "{qualified} stream request is missing {field}"
        );
    }
    let response = pool.message(method.output());
    let required_response_fields: &[&str] = if watch {
        &["sequence", "committed_cursor"]
    } else {
        &["sequence", "contents", "committed_cursor"]
    };
    for field in required_response_fields {
        assert!(
            response.field_by_name(field).is_some(),
            "{qualified} stream response is missing {field}"
        );
    }
    if watch {
        assert!(
            response.oneofs().iter().any(|oneof| oneof.name() == "item"),
            "{qualified} has no typed stream item"
        );
    }
}

#[test]
fn every_method_declares_auth_kind_limits_and_retry_policy() {
    let pool = pool();
    let mut methods = 0;

    for service in pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
    {
        for method in service.methods() {
            methods += 1;
            let qualified = format!("{}/{}", service.full_name(), method.name());
            let options = method.options().expect("every method has options");
            let authorization = options
                .extension(&AUTHORIZATION)
                .unwrap_or_else(|| panic!("{qualified} has no authorization policy"));
            assert!(
                !authorization.permission.is_empty(),
                "{qualified} has an empty permission"
            );
            assert_eq!(authorization.audience, format!("/{qualified}"));

            let bootstrap = qualified == "hephaestus.identity.v1.IdentityService/ResolveIdentity";
            assert_eq!(
                authorization.actor_source.to_i32(),
                if bootstrap {
                    OIDC_BOOTSTRAP
                } else {
                    MEDIATOR_JWT
                },
                "{qualified} has the wrong trusted actor source"
            );

            let operation = options
                .extension(&OPERATION_KIND)
                .unwrap_or_else(|| panic!("{qualified} has no operation kind"));
            assert!(
                options
                    .extension(&MAX_REQUEST_BYTES)
                    .is_some_and(|size| size > 0),
                "{qualified} has no positive request limit"
            );
            assert!(
                options
                    .extension(&MAX_RESPONSE_BYTES)
                    .is_some_and(|size| size > 0),
                "{qualified} has no positive response limit"
            );

            match operation {
                QUERY => assert!(
                    options.idempotency_level.is_some(),
                    "{qualified} query is missing NO_SIDE_EFFECTS"
                ),
                MUTATION => validate_mutation_method(&pool, method, &qualified),
                SERVER_STREAM => validate_server_stream_method(&pool, method, &qualified),
                _ => panic!("{qualified} has an unspecified operation kind"),
            }
        }
    }

    assert_eq!(methods, 51, "review the policy when adding an RPC method");
}

#[test]
fn mutation_receipt_has_exact_scoped_event_position() {
    let pool = pool();
    let receipt = pool
        .message_by_name("hephaestus.common.v1.MutationReceipt")
        .expect("shared mutation receipt");
    assert_eq!(
        receipt
            .fields()
            .iter()
            .map(buffa_descriptor::FieldDescriptor::name)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["committed_cursor", "aggregate_version", "event_id"])
    );
    assert!(message_field_is(
        &pool,
        receipt,
        "committed_cursor",
        "hephaestus.common.v1.Cursor"
    ));
    assert!(message_field_is(
        &pool,
        receipt,
        "event_id",
        "hephaestus.common.v1.OpaqueId"
    ));
    assert!(
        receipt
            .field_by_name("aggregate_version")
            .is_some_and(|field| matches!(
                field.kind(),
                FieldKind::Singular(SingularKind::Scalar(ScalarType::Uint64))
            ))
    );
}

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

    let coverage: ReducerCoverage =
        toml::from_str(include_str!("../../../proto/event-reducer-coverage.toml"))
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
    assert_eq!(coverage.variants.len(), 16);
}

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
    let contract = include_str!("../../../ARCHITECTURE.md");
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

#[test]
fn collections_are_paginated_with_stable_ordering() {
    let pool = pool();
    let page_response = pool
        .message_by_name("hephaestus.common.v1.PageResponse")
        .expect("shared page response");
    assert!(page_response.field_by_name("next_page_token").is_some());
    assert!(page_response.field_by_name("stable_order").is_some());

    for service in pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
    {
        for method in service.methods() {
            let output = pool.message(method.output());
            let collection_fields = output
                .fields()
                .iter()
                .filter(|field| matches!(field.kind(), FieldKind::List(_)))
                .collect::<Vec<_>>();
            if method.name().starts_with("List") || !collection_fields.is_empty() {
                let input = pool.message(method.input());
                let qualified = format!("{}/{}", service.full_name(), method.name());
                for collection_field in &collection_fields {
                    let page_field = if collection_fields.len() == 1 {
                        "page".to_owned()
                    } else {
                        format!("{}_page", collection_field.name())
                    };
                    assert!(
                        message_field_is(
                            &pool,
                            input,
                            &page_field,
                            "hephaestus.common.v1.PageRequest"
                        ),
                        "{qualified} collection {} is not paginated",
                        collection_field.name()
                    );
                    assert!(
                        message_field_is(
                            &pool,
                            output,
                            &page_field,
                            "hephaestus.common.v1.PageResponse"
                        ),
                        "{qualified} collection {} has no stable-order page metadata",
                        collection_field.name()
                    );
                }
            }
        }
    }
}

#[test]
fn application_payloads_are_typed_and_responses_are_secret_safe() {
    let pool = pool();
    let allowed_bytes = BTreeSet::from([
        "hephaestus.artifact.v1.StreamArtifactResponse.contents",
        "hephaestus.repository_browser.v1.StreamFileResponse.contents",
        "hephaestus.secret.v1.SecretValue.value",
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
        BTreeSet::from(["hephaestus.secret.v1.SecretValue.value".to_owned()])
    );
    for service in pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
    {
        for method in service.methods() {
            let mut visited = BTreeSet::new();
            assert_eq!(
                reachable_sensitive_field(&pool, pool.message(method.output()), &mut visited),
                None,
                "{}/{} exposes a sensitive field",
                service.full_name(),
                method.name()
            );
        }
    }
}

#[test]
fn sensitive_fields_are_annotated_request_only_and_error_safe() {
    let pool = pool();
    let sensitive = sensitive_fields(&pool);
    assert_eq!(
        sensitive.len(),
        1,
        "review every sensitive descriptor field"
    );
    for qualified in sensitive {
        let (message_name, field_name) = qualified
            .rsplit_once('.')
            .expect("sensitive field is qualified");
        let message = pool
            .message_by_name(message_name)
            .expect("sensitive field message");
        let field = message
            .field_by_name(field_name)
            .expect("sensitive field descriptor");
        assert!(matches!(
            field.kind(),
            FieldKind::Singular(SingularKind::Scalar(ScalarType::Bytes))
        ));
        let request_roots = pool
            .services()
            .iter()
            .filter(|service| service.full_name().starts_with("hephaestus."))
            .flat_map(|service| {
                service
                    .methods()
                    .iter()
                    .map(|method| pool.message(method.input()))
            })
            .collect::<Vec<_>>();
        assert!(
            request_roots.iter().any(|request| {
                message_reaches_named_message(&pool, request, message_name, &mut BTreeSet::new())
            }),
            "{qualified} is not reachable from a request"
        );
    }

    for error_message in [
        "hephaestus.common.v1.ErrorDetail",
        "hephaestus.common.v1.Diagnostic",
        "hephaestus.common.v1.RuntimeMetric",
    ] {
        let message = pool
            .message_by_name(error_message)
            .expect("error/observability descriptor");
        assert_eq!(
            reachable_sensitive_field(&pool, message, &mut BTreeSet::new()),
            None,
            "{error_message} can carry sensitive material"
        );
        for field in message.fields() {
            assert!(
                !contains_forbidden_sensitive_name(field.name()),
                "{error_message}.{} has a sensitive output name",
                field.name()
            );
            assert!(
                !matches!(
                    field.kind(),
                    FieldKind::Singular(SingularKind::Scalar(ScalarType::Bytes))
                ),
                "{error_message}.{} exposes opaque bytes",
                field.name()
            );
        }
    }
}

#[test]
fn actor_identity_is_metadata_only_except_for_exact_bootstrap_shape() {
    let pool = pool();
    for service in pool
        .services()
        .iter()
        .filter(|service| service.full_name().starts_with("hephaestus."))
    {
        for method in service.methods() {
            let qualified = format!("{}/{}", service.full_name(), method.name());
            let request = pool.message(method.input());
            if qualified == "hephaestus.identity.v1.IdentityService/ResolveIdentity" {
                let actual = request
                    .fields()
                    .iter()
                    .map(buffa_descriptor::FieldDescriptor::name)
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    actual,
                    BTreeSet::from([
                        "context",
                        "issuer",
                        "subject",
                        "display_name",
                        "email",
                        "email_verified",
                    ])
                );
            } else {
                assert!(
                    request
                        .fields()
                        .iter()
                        .all(|field| !field.name().contains("actor")),
                    "{qualified} contains a caller-selectable actor field"
                );
                assert_eq!(
                    reachable_actor_field(&pool, request, &mut BTreeSet::new()),
                    None,
                    "{qualified} hides a caller-selectable actor field in a nested message"
                );
            }
        }
    }

    let contract = include_str!("../../../proto/README.md");
    for required in [
        "hephaestus-rpc-mediator-v1\\0",
        "actor_kind",
        "verified_oidc_bootstrap",
        "oidc_iss",
        "oidc_sub",
        "email_verified",
        "no more than 30 seconds",
        "five seconds of clock skew",
    ] {
        assert!(
            contract.contains(required),
            "bootstrap contract omits {required}"
        );
    }
}

#[test]
fn descriptor_policy_fixtures_cover_sensitive_and_actor_failures() {
    let valid = include_str!("fixtures/descriptor-policy/valid/request_sensitive.proto");
    assert!(valid.contains("sensitive) = true"));
    assert!(!valid.contains("plaintext"));

    let invalid_output = include_str!("fixtures/descriptor-policy/invalid/sensitive_output.proto");
    assert!(contains_forbidden_sensitive_name("plaintext"));
    assert!(invalid_output.contains("bytes plaintext"));
    assert!(!invalid_output.contains("sensitive) = true"));

    let invalid_actor = include_str!("fixtures/descriptor-policy/invalid/actor_request.proto");
    assert!(invalid_actor.contains("string actor"));
}
