use buffa::ExtensionSet as _;
use buffa_descriptor::{FieldKind, ScalarType, SingularKind};
use rpc_proto::messages::hephaestus::options::v1::{AUTHORIZATION, SENSITIVE};
use std::collections::BTreeSet;

use super::common::{message_field_is, pool};

#[test]
fn project_service_log_metadata_uses_project_read_and_a_bounded_shape() {
    let pool = pool();
    let service = pool
        .service_by_name("hephaestus.gateway.v1.GatewayService")
        .expect("gateway service");
    let method = service
        .methods()
        .iter()
        .find(|method| method.name() == "GetProjectServiceLogMetadata")
        .expect("project service-log metadata method");
    let qualified = format!("{}/{}", service.full_name(), method.name());
    let authorization = method
        .options()
        .and_then(|options| options.extension(&AUTHORIZATION))
        .expect("project service-log metadata authorization");
    assert_eq!(authorization.permission, "project.read");
    assert_eq!(authorization.audience, format!("/{qualified}"));

    let request = pool.message(method.input());
    assert!(message_field_is(
        &pool,
        request,
        "project_id",
        "hephaestus.common.v1.OpaqueId"
    ));
    let response = pool.message(method.output());
    assert!(message_field_is(
        &pool,
        response,
        "metadata",
        "hephaestus.gateway.v1.GatewayServiceLogProjectMetadata"
    ));
    let metadata = pool
        .message_by_name("hephaestus.gateway.v1.GatewayServiceLogProjectMetadata")
        .expect("project service-log metadata message");
    for field in [
        "usage_present",
        "storage_dropped_chunks",
        "storage_dropped_bytes",
    ] {
        assert!(metadata.field_by_name(field).is_some(), "missing {field}");
    }
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
                if qualified == "hephaestus.gateway.v1.GatewayService/ListGatewayServiceLogs" {
                    assert!(
                        message_field_is(&pool, input, "after", "hephaestus.common.v1.Cursor"),
                        "{qualified} must use its scope-bound cursor"
                    );
                    assert!(
                        message_field_is(
                            &pool,
                            output,
                            "next_after",
                            "hephaestus.common.v1.Cursor"
                        ),
                        "{qualified} must return its scope-bound cursor"
                    );
                    continue;
                }
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
fn gateway_revision_service_declaration_is_optional_and_immutable() {
    let pool = pool();
    let revision = pool
        .message_by_name("hephaestus.gateway.v1.GatewayRevision")
        .expect("gateway revision descriptor");
    let service = revision
        .field_by_name("service")
        .expect("optional service declaration");
    assert_eq!(service.number(), 10);
    assert!(message_field_is(
        &pool,
        revision,
        "service",
        "hephaestus.gateway.v1.GatewayServiceDeclaration"
    ));

    let declaration = pool
        .message_by_name("hephaestus.gateway.v1.GatewayServiceDeclaration")
        .expect("service declaration descriptor");
    assert_eq!(
        declaration
            .fields()
            .iter()
            .map(buffa_descriptor::FieldDescriptor::name)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "loopback_port",
            "readiness_path",
            "health_path",
            "log_capture_mode"
        ])
    );
    assert!(declaration.fields().iter().all(|field| {
        field
            .options()
            .and_then(|options| options.extension(&SENSITIVE))
            != Some(true)
    }));

    let mode = pool
        .enum_by_name("hephaestus.gateway.v1.GatewayServiceLogCaptureMode")
        .expect("service log capture mode descriptor");
    assert_eq!(
        mode.values()
            .iter()
            .map(buffa_descriptor::EnumValueDescriptor::name)
            .collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "GATEWAY_SERVICE_LOG_CAPTURE_MODE_UNSPECIFIED",
            "GATEWAY_SERVICE_LOG_CAPTURE_MODE_DISABLED",
            "GATEWAY_SERVICE_LOG_CAPTURE_MODE_APPLICATION",
        ])
    );
}
