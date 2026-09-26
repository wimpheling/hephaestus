use super::RunRpc;
use super::auth::parse_id;
use super::conversions::opaque;
use super::errors::map_error;
use crate::application::run::{
    ControlKind as AppKind, ControlTarget as AppTarget, RequestControl as AppControl,
};
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::run::v1::{
    ControlState, RequestControlRequest, RequestControlResponse, RunControlKind, run_control_target,
};

pub(super) async fn request_control(
    rpc: &RunRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, RequestControlRequest>,
) -> ServiceResult<RequestControlResponse> {
    let budget = request::RequestBudget::from_transport(&ctx);
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &rpc.authenticator,
        "/hephaestus.run.v1.RunService/RequestControl",
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    if request.reason.len() > 4096 {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let kind = match request.kind.as_known() {
        Some(RunControlKind::Cancel) => AppKind::Cancel,
        Some(RunControlKind::Retry) => AppKind::Retry,
        Some(RunControlKind::ApproveResult) => AppKind::Approve,
        Some(RunControlKind::RejectResult) => AppKind::Reject,
        _ => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    let target = match request
        .target
        .as_option()
        .and_then(|target| target.target.as_ref())
    {
        Some(run_control_target::Target::RunId(id)) => {
            AppTarget::Run(id.value.parse().map_err(super::errors::invalid)?)
        }
        Some(run_control_target::Target::ProposalId(id)) => {
            AppTarget::Proposal(id.value.parse().map_err(super::errors::invalid)?)
        }
        None => return Err(into_connect_error(RpcError::InvalidArgument)),
    };
    if matches!(kind, AppKind::Cancel | AppKind::Retry) != matches!(target, AppTarget::Run(_)) {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let receipt_aggregate = if matches!(target, AppTarget::Run(_)) {
        "run"
    } else {
        "review"
    };
    let result = request::run_with_budget(
        &budget,
        rpc.application.request_control(
            &identity,
            AppControl {
                kind,
                repository_id: parse_id(request.repository_id.as_option())?,
                target,
                reason: request.reason,
            },
        ),
    )
    .await
    .map_err(into_connect_error)?
    .map_err(map_error)?;
    let state = match result.state.as_str() {
        "pending" | "processing" => ControlState::Queued,
        "completed" => ControlState::Applied,
        "failed" => ControlState::Rejected,
        _ => return Err(into_connect_error(RpcError::Internal)),
    };
    let receipt = request::run_with_budget(
        &budget,
        mutation_receipt(
            &rpc.receipts,
            identity.idempotency_id,
            identity.user_id,
            receipt_aggregate,
            "run",
        ),
    )
    .await
    .map_err(into_connect_error)??;
    Response::ok(RequestControlResponse {
        control_request_id: opaque(result.id).into(),
        state: state.into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}
