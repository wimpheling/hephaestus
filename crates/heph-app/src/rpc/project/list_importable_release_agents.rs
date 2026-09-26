mod mapping;

use super::{ProjectRpc, map_error, opaque, parse_id, parse_page};
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use mapping::release_agent_option;
use rpc_proto::messages::hephaestus::{
    common::v1::PageResponse,
    project::v1::{ListImportableReleaseAgentsRequest, ListImportableReleaseAgentsResponse},
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListImportableReleaseAgentsRequest>,
) -> ServiceResult<ListImportableReleaseAgentsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/ListImportableReleaseAgents",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_id(request.project_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = request::run_with_budget(
        &budget,
        service
            .application
            .importable_agents(&identity, project_id, page),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(map_error)
    .map_err(into_connect_error)?;
    let release_agents = result
        .values
        .into_iter()
        .map(release_agent_option)
        .collect::<Result<Vec<_>, crate::rpc::RpcError>>()
        .map_err(into_connect_error)?;
    Response::ok(ListImportableReleaseAgentsResponse {
        release_agents,
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::mapping::{
        capability_requirements, parameter_schema, runtime_contract, secret_slot_schema,
    };
    use rpc_proto::messages::hephaestus::common::v1::{
        NetworkPolicy, parameter_default, parameter_type,
    };
    use serde_json::json;

    #[test]
    fn durable_release_documents_preserve_typed_defaults_and_lists() {
        let parameters = parameter_schema(&json!([
            {
                "name": "review_style",
                "value_type": {"type": "enum", "values": ["strict", "balanced"]},
                "required": true,
                "default": "balanced",
                "sensitive": false
            },
            {
                "name": "attempts",
                "value_type": {"type": "integer", "minimum": 1, "maximum": 4},
                "required": false,
                "default": 2,
                "sensitive": false
            },
            {
                "name": "enabled",
                "value_type": {"type": "boolean"},
                "required": false,
                "default": true,
                "sensitive": false
            }
        ]))
        .expect("durable parameter schema");

        assert_eq!(parameters[0].name, "review_style");
        assert!(matches!(
            parameters[0]
                .value_type
                .as_option()
                .and_then(|value| value.constraint.as_ref()),
            Some(parameter_type::Constraint::Enumeration(value))
                if value.values == ["strict", "balanced"]
        ));
        assert!(matches!(
            parameters[0]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::StringValue(value)) if value == "balanced"
        ));
        assert!(matches!(
            parameters[1]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::IntegerValue(2))
        ));
        assert!(matches!(
            parameters[2]
                .default
                .as_option()
                .and_then(|value| value.value.as_ref()),
            Some(parameter_default::Value::BooleanValue(true))
        ));

        let slots = secret_slot_schema(&json!([{
            "key": "review_token",
            "purpose": "Review API",
            "required": true,
            "delivery_modes": ["raw", "brokered"],
            "phases": ["normal", "update"],
            "destinations": ["reviews.submit"]
        }]))
        .expect("durable secret slot schema");
        assert_eq!(slots[0].delivery_modes.len(), 2);
        assert_eq!(slots[0].phases.len(), 2);
        assert_eq!(slots[0].destinations, ["reviews.submit"]);

        let contract = runtime_contract(
            &json!({
                "policy_ceiling": {
                    "vcpus": 1,
                    "memory_mib": 128,
                    "network": "broker_only"
                }
            }),
            true,
        )
        .expect("durable runtime contract");
        assert_eq!(
            contract
                .policy_ceiling
                .as_option()
                .map(|policy| policy.network),
            Some(NetworkPolicy::BrokerOnly.into())
        );
        assert!(contract.requires_state);

        let requirements = capability_requirements(&json!([{
            "id": "c68b4174-8438-4a38-bca3-922040176226",
            "release_agent_id": "f03a4989-391c-41e1-95ee-f5567909e798",
            "slot_key": "source_repository",
            "purpose": "Read source and publish reviewed changes",
            "resource_kind": "repository",
            "required_operations": ["git_read"],
            "optional_operations": ["update_ref"],
            "slot_required": true
        }]))
        .expect("durable capability requirements");
        assert_eq!(requirements[0].slot_key, "source_repository");
        assert_eq!(requirements[0].required_operations, ["git_read"]);
        assert_eq!(requirements[0].optional_operations, ["update_ref"]);
        assert!(requirements[0].slot_required);
    }
}
