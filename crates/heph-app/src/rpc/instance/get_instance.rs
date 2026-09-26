use super::InstanceRpc;
use crate::{
    application::instance::InstanceQueryError,
    rpc::{RpcError, into_connect_error, request},
};
use buffa::Message as _;
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::instance::v1::{GetInstanceRequest, GetInstanceResponse};

mod contract;
mod model;
mod policy;
#[cfg(test)]
#[path = "get_instance/tests.rs"]
mod tests;

const MAX_RESPONSE_BYTES: u32 = 4 * 1_048_576;
const MAX_HOOK_EVENTS: usize = 500;

pub(super) async fn handle(
    service: &InstanceRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, GetInstanceRequest>,
) -> ServiceResult<GetInstanceResponse> {
    let identity = request::query_identity(
        &ctx,
        &service.authenticator,
        "/hephaestus.instance.v1.AgentInstanceService/GetInstance",
    )
    .map_err(into_connect_error)?;
    let instance_id = request::required_id(message.to_owned_message().instance_id.as_option())
        .and_then(|value| value.parse().map_err(|_| RpcError::InvalidArgument))
        .map_err(into_connect_error)?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let snapshot =
        request::run_with_budget(&budget, service.application.get(&identity, instance_id))
            .await
            .map_err(into_connect_error)?
            .map_err(map_error)
            .map_err(into_connect_error)?;
    let response = GetInstanceResponse {
        instance: model::project(snapshot).map_err(into_connect_error)?.into(),
        ..Default::default()
    };
    ensure_response_bound(&response).map_err(into_connect_error)?;
    Response::ok(response)
}

pub(super) fn ensure_response_bound(response: &GetInstanceResponse) -> Result<(), RpcError> {
    if response.encoded_len() > MAX_RESPONSE_BYTES {
        Err(RpcError::ResourceExhausted)
    } else {
        Ok(())
    }
}

fn map_error(error: InstanceQueryError) -> RpcError {
    match error {
        InstanceQueryError::PermissionDenied => RpcError::PermissionDenied,
        InstanceQueryError::NotFound => RpcError::NotFound,
        InstanceQueryError::ResponseTooLarge => RpcError::ResourceExhausted,
        InstanceQueryError::Persistence(source) => {
            tracing::error!(error = %source, "agent-instance query failed");
            RpcError::Unavailable
        }
    }
}
