use buffa::ExtensionSet as _;
use buffa_descriptor::DescriptorPool;
use rpc_proto::messages::hephaestus::options::v1::{
    AUTHORIZATION, MAX_REQUEST_BYTES, MAX_RESPONSE_BYTES, OPERATION_KIND,
};
use std::collections::BTreeSet;

use super::common::{message_field_is, pool};

const QUERY: i32 = 1;
const MUTATION: i32 = 2;
const SERVER_STREAM: i32 = 3;
const MEDIATOR_JWT: i32 = 1;
const OIDC_BOOTSTRAP: i32 = 2;

#[test]
fn reflection_inventory_contains_every_application_service_and_method() {
    let pool = pool();
    let expected = BTreeSet::from([
        "hephaestus.artifact.v1.ArtifactService",
        "hephaestus.build.v1.BuildService",
        "hephaestus.image.v1.ImageCatalogService",
        "hephaestus.event.v1.ProductEventService",
        "hephaestus.gateway.v1.GatewayService",
        "hephaestus.identity.v1.IdentityService",
        "hephaestus.instance.v1.AgentInstanceService",
        "hephaestus.organization.v1.OrganizationService",
        "hephaestus.pat.v1.PersonalAccessTokenService",
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
        98
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
    // Browser handoff is an ephemeral capability issue. It deliberately
    // rejects idempotency keys and has no durable mutation receipt ledger;
    // its request correlation remains in the authenticated audit path.
    if qualified != "hephaestus.release.v1.ReleaseService/CreateUiBrowserHandoff" {
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

            let bootstrap = matches!(
                qualified.as_str(),
                "hephaestus.identity.v1.IdentityService/ResolveIdentity"
                    | "hephaestus.identity.v1.IdentityService/CreateBrowserSession"
            );
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

    assert_eq!(methods, 98, "review the policy when adding an RPC method");
}
