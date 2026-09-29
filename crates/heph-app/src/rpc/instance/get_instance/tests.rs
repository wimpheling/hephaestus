use super::contract::{events, network_restriction, parameter_schema, selector};
use super::{MAX_HOOK_EVENTS, MAX_RESPONSE_BYTES, ensure_response_bound};
use crate::rpc::{RpcError, request};
use rpc_proto::messages::hephaestus::{
    common::v1::{NetworkPolicy, parameter_type},
    instance::v1::{AgentInstance, GetInstanceResponse, ref_selector, update_event},
};
use serde_json::json;
use std::time::{Duration, Instant};

#[test]
fn stored_documents_become_generated_contract_types() {
    let declarations = parameter_schema(&json!([{
        "name": "count",
        "value_type": {"type": "integer", "minimum": 1, "maximum": 4},
        "required": true,
        "sensitive": false
    }]))
    .expect("parameter schema");
    let constraint = declarations[0]
        .value_type
        .as_option()
        .and_then(|value| value.constraint.as_ref())
        .expect("parameter constraint");
    assert!(matches!(
        constraint,
        parameter_type::Constraint::Integer(value)
            if value.minimum == 1 && value.maximum == 4
    ));

    let selector = selector("refs/heads/*");
    assert!(matches!(
        selector.selector,
        Some(ref_selector::Selector::Prefix(value)) if value == "refs/heads"
    ));
    assert_eq!(
        network_restriction(&json!({"network": "broker_only"})).expect("network"),
        NetworkPolicy::BrokerOnly
    );
}

#[test]
fn update_logs_are_bounded_to_the_contract_limit() {
    let parsed_events = events(
        &json!([{"sequence":1,"event_type":"vm.log","payload":{"message":"x".repeat(5000)}}]),
    )
    .expect("events");
    match parsed_events[0].payload.as_ref().expect("payload") {
        update_event::Payload::BoundedLogMessage(value) => assert_eq!(value.len(), 4096),
        _ => panic!("unexpected event"),
    }

    let excessive = serde_json::Value::Array(
        (0..=MAX_HOOK_EVENTS)
            .map(|sequence| {
                json!({"sequence": sequence, "event_type": "operation.state", "payload": {"state": "running"}})
            })
            .collect(),
    );
    assert!(events(&excessive).is_err());
}

#[test]
fn response_enforces_the_generated_four_mebibyte_limit() {
    let response = GetInstanceResponse {
        instance: AgentInstance {
            name: "x".repeat(usize::try_from(MAX_RESPONSE_BYTES).expect("response bound") + 1),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    assert!(ensure_response_bound(&response).is_err());
}

#[tokio::test]
async fn get_instance_budget_stops_an_expired_query() {
    let budget = request::RequestBudget::from_deadline(Some(
        Instant::now()
            .checked_sub(Duration::from_millis(1))
            .expect("instant supports subtraction"),
    ));
    let result = request::run_with_budget(&budget, std::future::pending::<()>()).await;
    assert_eq!(result, Err(RpcError::DeadlineExceeded));
}
