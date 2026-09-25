use super::GatewayRpc;
use crate::rpc::{RpcError, into_connect_error, mutation_receipt, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use gateway_postgres::{ConfigureGatewayRequest, GatewayConfigureError, GatewaySecretSelection};
use release_domain::{ParameterName, ParameterValue};
use rpc_proto::messages::hephaestus::{
    common::v1::{ParameterValue as ProtoParameterValue, parameter_value},
    gateway::v1::{ConfigureGatewayRequest as ProtoRequest, ConfigureGatewayResponse},
};
use std::collections::BTreeMap;

const AUDIENCE: &str = "/hephaestus.gateway.v1.GatewayService/ConfigureGateway";

pub(super) async fn handle(
    service: &GatewayRpc,
    ctx: RequestContext,
    message: ServiceRequest<'_, ProtoRequest>,
) -> ServiceResult<ConfigureGatewayResponse> {
    let request = message.to_owned_message();
    let identity = request::mutation_identity(
        &ctx,
        &service.authenticator,
        AUDIENCE,
        request.context.as_option(),
    )
    .map_err(into_connect_error)?;
    let command = ConfigureGatewayRequest {
        gateway_id: super::id(request.gateway_id.as_option())?,
        expected_revision_id: super::id(request.expected_revision_id.as_option())?,
        parameters: parameters(request.parameters).map_err(into_connect_error)?,
        secret_selections: request
            .secret_selections
            .into_iter()
            .map(|selection| {
                Ok(GatewaySecretSelection {
                    slot_key: selection.slot_key,
                    import_id: super::id(selection.import_id.as_option())?,
                    secret_version_id: super::id(selection.secret_version_id.as_option())?,
                    route_path: selection.route_path,
                    header_name: selection.header_name,
                })
            })
            .collect::<Result<_, connectrpc::ConnectError>>()?,
    };
    let result = service
        .application
        .configure(&identity, command)
        .await
        .map_err(|error| map_error(&error))
        .map_err(into_connect_error)?;
    let receipt = mutation_receipt(
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "gateway",
        "project",
    )
    .await?;
    Response::ok(ConfigureGatewayResponse {
        revision_id: super::opaque(result.revision_id).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

fn parameters(
    values: Vec<ProtoParameterValue>,
) -> Result<BTreeMap<ParameterName, ParameterValue>, RpcError> {
    let mut result = BTreeMap::new();
    for value in values {
        let name = ParameterName::parse(value.name).map_err(|_| RpcError::InvalidArgument)?;
        let value = match value.value {
            Some(parameter_value::Value::StringValue(value)) => ParameterValue::String(value),
            Some(parameter_value::Value::IntegerValue(value)) => ParameterValue::Integer(value),
            Some(parameter_value::Value::BooleanValue(value)) => ParameterValue::Boolean(value),
            None => return Err(RpcError::InvalidArgument),
        };
        if result.insert(name, value).is_some() {
            return Err(RpcError::InvalidArgument);
        }
    }
    Ok(result)
}

const fn map_error(error: &GatewayConfigureError) -> RpcError {
    match error {
        GatewayConfigureError::Denied => RpcError::PermissionDenied,
        GatewayConfigureError::NotFound => RpcError::NotFound,
        GatewayConfigureError::InvalidArgument => RpcError::InvalidArgument,
        GatewayConfigureError::Stale => RpcError::FailedPrecondition,
        GatewayConfigureError::Conflict => RpcError::AlreadyExists,
        GatewayConfigureError::Persistence(_) => RpcError::Unavailable,
    }
}
