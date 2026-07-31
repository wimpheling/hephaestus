use super::{RepositoryRpc, map_error, opaque, parse_id, timestamp};
use crate::{
    application::repository::RunRow,
    rpc::{RpcError, into_connect_error, request},
};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::{
    common::v1::{Diagnostic, DiagnosticCode, DiagnosticSeverity},
    repository::v1::{GetRepositoryRequest, GetRepositoryResponse, Repository, RepositoryRun},
};

pub(super) async fn handle(
    service: &RepositoryRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetRepositoryRequest>,
) -> ServiceResult<GetRepositoryResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.repository.v1.RepositoryService/GetRepository",
    )
    .map_err(into_connect_error)?;
    let repository_id = parse_id(message.to_owned_message().repository_id.as_option())
        .map_err(into_connect_error)?;
    let result = service
        .application
        .get(&identity, repository_id)
        .await
        .map_err(map_error)
        .map_err(into_connect_error)?;
    let runs = result
        .runs
        .into_iter()
        .map(repository_run)
        .collect::<Result<Vec<_>, RpcError>>()
        .map_err(into_connect_error)?;
    let row = result.repository;
    Response::ok(GetRepositoryResponse {
        repository: Repository {
            id: opaque(row.id).into(),
            name: row.name,
            default_branch: row.default_branch,
            is_public: row.is_public,
            project_id: opaque(row.project_id).into(),
            project_name: row.project_name,
            organization_id: opaque(row.organization_id).into(),
            organization_name: row.organization_name,
            runs,
            ..Default::default()
        }
        .into(),
        ..Default::default()
    })
}

fn repository_run(row: RunRow) -> Result<RepositoryRun, RpcError> {
    Ok(RepositoryRun {
        id: opaque(row.id).into(),
        state: row.state,
        outcome: row.outcome.unwrap_or_default(),
        exit_code: row.exit_code,
        failure: row
            .failure
            .as_deref()
            .map(failure_diagnostic)
            .into_iter()
            .collect(),
        created_at: timestamp(row.created_at).into(),
        updated_at: timestamp(row.updated_at).into(),
        agent_name: row.agent_name,
        commit_sha: row.commit_sha,
        git_ref: row.git_ref,
        attempt: u32::try_from(row.attempt).map_err(|_| RpcError::Internal)?,
        proposal_id: row.proposal_id.map(opaque).into(),
        proposal_state: row.proposal_state.unwrap_or_default(),
        ..Default::default()
    })
}

fn failure_diagnostic(message: &str) -> Diagnostic {
    Diagnostic {
        code: DiagnosticCode::ResourceUnavailable.into(),
        severity: DiagnosticSeverity::Error.into(),
        message: message.chars().take(4096).collect(),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::repository_run;
    use crate::application::repository::RunRow;
    use rpc_proto::messages::hephaestus::common::v1::{DiagnosticCode, DiagnosticSeverity};

    #[test]
    fn stored_run_failure_projects_to_one_bounded_diagnostic() {
        let created_at = time::OffsetDateTime::from_unix_timestamp(1_700_000_000)
            .expect("valid fixture timestamp");
        let projected = repository_run(RunRow {
            id: uuid::Uuid::new_v4(),
            state: String::from("cleaned_up"),
            outcome: Some(String::from("failed")),
            exit_code: Some(1),
            failure: Some("x".repeat(5_000)),
            created_at,
            updated_at: created_at + time::Duration::seconds(1),
            agent_name: String::from("reviewer"),
            commit_sha: "a".repeat(40),
            git_ref: String::from("refs/heads/main"),
            attempt: 1,
            proposal_id: None,
            proposal_state: None,
        })
        .expect("stored run should project");

        assert_eq!(projected.failure.len(), 1);
        assert_eq!(
            projected.failure[0].code,
            DiagnosticCode::ResourceUnavailable
        );
        assert_eq!(projected.failure[0].severity, DiagnosticSeverity::Error);
        assert_eq!(projected.failure[0].message.len(), 4096);
        assert_eq!(
            projected.created_at.as_option().map(|value| value.seconds),
            Some(created_at.unix_timestamp())
        );
    }
}
