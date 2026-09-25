use super::RunRpc;
use super::auth::{parse_id, parse_page, query};
use super::conversions::{opaque, proto_run, timestamp};
use super::errors::map_error;
use crate::rpc::{into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::common::v1::PageResponse;
use rpc_proto::messages::hephaestus::run::v1::{
    GetRunRequest, GetRunResponse, ListProjectRunsRequest, ListProjectRunsResponse, RunSummary,
};

pub(super) async fn list_project_runs(
    rpc: &RunRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ListProjectRunsRequest>,
) -> ServiceResult<ListProjectRunsResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &rpc.authenticator, "ListProjectRuns")?;
    let request = message.to_owned_message();
    let result = request::run_with_budget(
        &budget,
        rpc.application.list_project_runs(
            &identity,
            parse_id(request.project_id.as_option())?,
            parse_page(request.page.as_option())?,
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(map_error)?;
    Response::ok(ListProjectRunsResponse {
        page: PageResponse {
            next_page_token: result.next.unwrap_or_default(),
            stable_order: String::from("created_at_desc,id_desc"),
            ..Default::default()
        }
        .into(),
        runs: result
            .values
            .into_iter()
            .map(|row| RunSummary {
                id: opaque(row.id).into(),
                state: row.state,
                outcome: row.outcome.unwrap_or_default(),
                run_kind: row.run_kind,
                updated_at: timestamp(row.updated_at).into(),
                instance_id: opaque(row.instance_id).into(),
                instance_name: row.instance_name,
                repository_id: row.repository_id.map(opaque).into(),
                repository_name: row.repository_name.unwrap_or_default(),
                commit_sha: row.commit_sha.unwrap_or_default(),
                git_ref: row.git_ref.unwrap_or_default(),
                release_id: opaque(row.release_id).into(),
                release_version: row.release_version,
                instance_revision_id: opaque(row.instance_revision_id).into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    })
}

pub(super) async fn get_run(
    rpc: &RunRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetRunRequest>,
) -> ServiceResult<GetRunResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let identity = query(&ctx, &rpc.authenticator, "GetRun")?;
    let request = message.to_owned_message();
    let run = request::run_with_budget(
        &budget,
        rpc.application
            .get_run(&identity, parse_id(request.run_id.as_option())?),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(map_error)?;
    Response::ok(GetRunResponse {
        run: proto_run(run)?.into(),
        ..Default::default()
    })
}
