use buffa_descriptor::{FieldKind, ScalarType, SingularKind};
use std::collections::BTreeSet;

use super::common::{
    contains_forbidden_sensitive_name, message_reaches_named_message, pool, reachable_actor_field,
    reachable_sensitive_field, sensitive_fields,
};

#[test]
fn sensitive_fields_are_annotated_and_reachable_only_at_reviewed_boundaries() {
    let pool = pool();
    let sensitive = sensitive_fields(&pool);
    assert_eq!(
        sensitive.len(),
        4,
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
        if qualified == "hephaestus.pat.v1.PersonalAccessTokenValue.value" {
            assert!(
                !request_roots.iter().any(|request| {
                    message_reaches_named_message(
                        &pool,
                        request,
                        message_name,
                        &mut BTreeSet::new(),
                    )
                }),
                "{qualified} must never be accepted in a request"
            );
        } else {
            assert!(
                request_roots.iter().any(|request| {
                    message_reaches_named_message(
                        &pool,
                        request,
                        message_name,
                        &mut BTreeSet::new(),
                    )
                }),
                "{qualified} is not reachable from a request"
            );
        }
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
            } else if qualified == "hephaestus.identity.v1.IdentityService/CreateBrowserSession" {
                let actual = request
                    .fields()
                    .iter()
                    .map(buffa_descriptor::FieldDescriptor::name)
                    .collect::<BTreeSet<_>>();
                assert_eq!(
                    actual,
                    BTreeSet::from(["context", "issuer", "subject", "sid"])
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

    let contract = include_str!("../../../../../../proto/README.md");
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
    let valid = include_str!("../fixtures/descriptor-policy/valid/request_sensitive.proto");
    assert!(valid.contains("sensitive) = true"));
    assert!(!valid.contains("plaintext"));

    let invalid_output =
        include_str!("../fixtures/descriptor-policy/invalid/sensitive_output.proto");
    assert!(contains_forbidden_sensitive_name("plaintext"));
    assert!(invalid_output.contains("bytes plaintext"));
    assert!(!invalid_output.contains("sensitive) = true"));

    let invalid_actor = include_str!("../fixtures/descriptor-policy/invalid/actor_request.proto");
    assert!(invalid_actor.contains("string actor"));
}
