use super::super::input;
use rpc_proto::messages::hephaestus::{
    common::v1::{OpaqueId, ParameterValue, parameter_value::Value},
    project::v1::{
        InstallRecipeDeploymentRequest, RecipeExternalVolumeBinding, ResumeRecipeDeploymentRequest,
    },
};
fn install() -> InstallRecipeDeploymentRequest {
    InstallRecipeDeploymentRequest { project_id: OpaqueId { value: uuid::Uuid::new_v4().to_string(), ..Default::default() }.into(),
        deployment_key: "bounded".into(), declaration_toml: r#"contract_version = 1
recipe_id = "application"
recipe_version = "1.0.0"
[[resources]]
kind = "instance"
name = "app"
removal = "delete"
release = { release_id = "10000000-0000-4000-8000-000000000001", release_agent_id = "10000000-0000-4000-8000-000000000002" }
"#.into(), ..Default::default() }
}
#[test]
fn duplicates_unknown_values_and_nil_bindings_are_denied_before_planning() {
    assert!(input::install(&install()).is_ok());
    let parameter = ParameterValue {
        name: "value".into(),
        value: Some(Value::BooleanValue(true)),
        ..Default::default()
    };
    let mut request = install();
    request.inputs = vec![parameter.clone(), parameter];
    assert!(input::install(&request).is_err());
    request.inputs = vec![ParameterValue {
        name: "value".into(),
        ..Default::default()
    }];
    assert!(input::install(&request).is_err());
    request.inputs.clear();
    request.external_volumes = vec![RecipeExternalVolumeBinding {
        name: "data".into(),
        volume_id: OpaqueId {
            value: uuid::Uuid::nil().to_string(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    }];
    assert!(input::install(&request).is_err());
    request.external_volumes[0].volume_id = OpaqueId {
        value: uuid::Uuid::new_v4().to_string(),
        ..Default::default()
    }
    .into();
    request
        .external_volumes
        .push(request.external_volumes[0].clone());
    assert!(input::install(&request).is_err());
}
#[test]
fn request_bounds_and_nil_resume_identifiers_fail_closed() {
    let mut request = install();
    request.declaration_toml = "x".repeat(65537);
    assert!(input::install(&request).is_err());
    request = install();
    request.deployment_key = "x".repeat(129);
    assert!(input::install(&request).is_err());
    request = install();
    request.inputs = vec![ParameterValue::default(); 65];
    assert!(input::install(&request).is_err());
    request = install();
    request.inputs = vec![ParameterValue {
        name: "value".into(),
        value: Some(Value::StringValue("x".repeat(4097))),
        ..Default::default()
    }];
    assert!(input::install(&request).is_err());
    let resume = ResumeRecipeDeploymentRequest {
        project_id: request.project_id,
        deployment_id: OpaqueId {
            value: uuid::Uuid::nil().to_string(),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    };
    assert!(input::resume(&resume).is_err());
}

#[test]
fn unknown_union_wire_and_unknown_request_fields_are_rejected() {
    let unknown = buffa::UnknownField {
        number: 99,
        data: buffa::UnknownFieldData::Varint(1),
    };
    let mut request = install();
    request.inputs = vec![ParameterValue {
        name: "value".into(),
        ..Default::default()
    }];
    request.inputs[0]
        .__buffa_unknown_fields
        .push(unknown.clone());
    assert!(input::install(&request).is_err());
    request = install();
    request.__buffa_unknown_fields.push(unknown);
    assert!(input::install(&request).is_err());
}
