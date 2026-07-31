use super::{ProjectRpc, map_error, opaque, parse_id, parse_page, timestamp};
use crate::{
    application::project::InstanceRow,
    rpc::{into_connect_error, request},
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::{Diagnostic, DiagnosticCode, DiagnosticSeverity, PageResponse},
    project::v1::{InstanceSummary, ListProjectInstancesRequest, ListProjectInstancesResponse},
};

pub(super) async fn handle(
    service: &ProjectRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectInstancesRequest>,
) -> ServiceResult<ListProjectInstancesResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.project.v1.ProjectService/ListProjectInstances",
    )
    .map_err(into_connect_error)?;
    let request = message.to_owned_message();
    let project_id = parse_id(request.project_id.as_option()).map_err(into_connect_error)?;
    let page = parse_page(request.page.as_option()).map_err(into_connect_error)?;
    let result = service
        .application
        .instances(&identity, project_id, page)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let instances = result
        .values
        .into_iter()
        .map(instance_summary)
        .collect::<Result<Vec<_>, crate::rpc::RpcError>>()
        .map_err(into_connect_error)?;
    Response::ok(ListProjectInstancesResponse {
        instances,
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("id"),
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

fn instance_summary(row: InstanceRow) -> Result<InstanceSummary, crate::rpc::RpcError> {
    Ok(InstanceSummary {
        id: opaque(row.id).into(),
        name: row.name,
        state: row.state,
        run_gate_open: row.run_gate_open,
        active_revision_id: opaque(row.active_revision_id).into(),
        state_volume_id: row.state_volume_id.map(opaque).into(),
        updated_at: timestamp(row.updated_at).into(),
        runnable: row.runnable.unwrap_or(false),
        platform_policy_version: row.platform_policy_version.unwrap_or_default(),
        diagnostics: diagnostics(row.diagnostics.as_ref())?,
        release_id: row.release_id.map(opaque).into(),
        release_version: row.release_version.unwrap_or_default(),
        release_state: row.release_state.unwrap_or_default(),
        release_agent_name: row.release_agent_name.unwrap_or_default(),
        attachment_count: row.attachment_count,
        run_count: row.run_count,
        last_run_at: row.last_run_at.map(timestamp).into(),
        ..Default::default()
    })
}

fn diagnostics(value: Option<&serde_json::Value>) -> Result<Vec<Diagnostic>, crate::rpc::RpcError> {
    value.map_or_else(
        || Ok(Vec::new()),
        |value| {
            value
                .as_array()
                .ok_or(crate::rpc::RpcError::Internal)
                .map(|values| values.iter().map(diagnostic).collect())
        },
    )
}

fn diagnostic(value: &serde_json::Value) -> Diagnostic {
    Diagnostic {
        code: diagnostic_code(value.get("code").and_then(serde_json::Value::as_str)).into(),
        severity: DiagnosticSeverity::Error.into(),
        field: value
            .get("field")
            .or_else(|| value.get("path"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        message: value
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .chars()
            .take(4096)
            .collect(),
        ..Default::default()
    }
}

fn diagnostic_code(code: Option<&str>) -> DiagnosticCode {
    match code {
        Some("invalid_parameter" | "invalid_default" | "invalid_value" | "required_missing") => {
            DiagnosticCode::InvalidParameter
        }
        Some("policy_exceeded") => DiagnosticCode::PolicyExceeded,
        Some(
            "incompatible_update"
            | "secret_binding_incompatible"
            | "state_capability_change_unsupported"
            | "stateful_update_hook_missing",
        ) => DiagnosticCode::IncompatibleUpdate,
        Some("run_gate_closed") => DiagnosticCode::RunGateClosed,
        Some(
            "resource_unavailable"
            | "required_secret_binding_missing"
            | "secret_binding_unavailable",
        ) => DiagnosticCode::ResourceUnavailable,
        _ => DiagnosticCode::Unspecified,
    }
}

#[cfg(test)]
mod tests {
    use super::{diagnostics, instance_summary};
    use crate::application::project::InstanceRow;
    use rpc_proto::messages::hephaestus::common::v1::{DiagnosticCode, DiagnosticSeverity};
    use serde_json::json;

    #[test]
    fn release_diagnostic_documents_are_projected_explicitly() {
        let projected = diagnostics(Some(&json!([
            {
                "code": "required_secret_binding_missing",
                "field": "secret_slots.api_token"
            },
            {
                "code": "stateful_update_hook_missing",
                "field": "update_hook"
            }
        ])))
        .expect("stored release diagnostics should project");

        assert_eq!(projected.len(), 2);
        assert_eq!(projected[0].code, DiagnosticCode::ResourceUnavailable);
        assert_eq!(projected[0].severity, DiagnosticSeverity::Error);
        assert_eq!(projected[0].field, "secret_slots.api_token");
        assert_eq!(projected[1].code, DiagnosticCode::IncompatibleUpdate);
        assert_eq!(projected[1].field, "update_hook");
    }

    #[test]
    fn nonempty_project_instance_row_projects_timestamp_and_release_documents() {
        let updated_at = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("valid fixture timestamp")
            + time::Duration::nanoseconds(123_456_789);
        let last_run_at = updated_at + time::Duration::minutes(5);
        let instance_id = uuid::Uuid::new_v4();
        let revision_id = uuid::Uuid::new_v4();
        let release_id = uuid::Uuid::new_v4();
        let projected = instance_summary(InstanceRow {
            id: instance_id,
            name: String::from("reviewer"),
            state: String::from("active"),
            run_gate_open: true,
            active_revision_id: revision_id,
            state_volume_id: None,
            updated_at,
            runnable: Some(false),
            platform_policy_version: Some(String::from("platform/v1")),
            diagnostics: Some(json!([{
                "code": "required_secret_binding_missing",
                "field": "secret_slots.api_token"
            }])),
            release_id: Some(release_id),
            release_version: Some(String::from("v1.0.0")),
            release_state: Some(String::from("published")),
            release_agent_name: Some(String::from("Reviewer")),
            attachment_count: 1,
            run_count: 2,
            last_run_at: Some(last_run_at),
        })
        .expect("realistic instance row should project");

        assert_eq!(
            projected.id.as_option().map(|id| id.value.clone()),
            Some(instance_id.to_string())
        );
        assert_eq!(
            projected
                .updated_at
                .as_option()
                .map(|value| (value.seconds, value.nanos)),
            Some((updated_at.unix_timestamp(), 123_456_789))
        );
        assert_eq!(
            projected.last_run_at.as_option().map(|value| value.seconds),
            Some(last_run_at.unix_timestamp())
        );
        assert_eq!(projected.diagnostics.len(), 1);
        assert_eq!(
            projected.release_id.as_option().map(|id| id.value.clone()),
            Some(release_id.to_string())
        );
    }
}
