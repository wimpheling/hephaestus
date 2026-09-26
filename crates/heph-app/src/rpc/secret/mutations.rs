use super::{
    budget::{execute, receipt},
    model::{
        delivery_modes, from_timestamp, invalid, json_id, mutation, opaque, parse_id, secret_owner,
        secret_policy, secret_target,
    },
};
use crate::application::commands::InternalCommand;
use crate::rpc::{RpcError, into_connect_error, request};
use connectrpc::{RequestContext, Response, ServiceRequest, ServiceResult};
use rpc_proto::messages::hephaestus::secret::v1::{
    AcceptSecretImportRequest, AcceptSecretImportResponse, CreateSecretRequest,
    CreateSecretResponse, GrantSecretRequest, GrantSecretResponse, PurgeSecretRequest,
    PurgeSecretResponse, RevokeSecretRequest, RevokeSecretResponse, RotateSecretRequest,
    RotateSecretResponse, SecretState, SetSecretEnabledRequest, SetSecretEnabledResponse,
};
use secret_domain::{SecretAlias, SecretName};

pub(super) async fn create_secret(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, CreateSecretRequest>,
) -> ServiceResult<CreateSecretResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "CreateSecret",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let secret = request
        .secret
        .as_option()
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?;
    let value = execute(
        service,
        &budget,
        &identity,
        InternalCommand::CreateSecret {
            owner: secret_owner(request.owner.as_option())?,
            name: SecretName::parse(request.name).map_err(invalid)?,
            allowed_delivery_modes: delivery_modes(request.allowed_delivery_modes)?,
            value: secret.value.clone(),
        },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_metadata",
        "organization",
    )
    .await?;
    Response::ok(CreateSecretResponse {
        secret_id: opaque(json_id(&value, "secret_id")?).into(),
        version_id: opaque(json_id(&value, "secret_version_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn rotate_secret(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RotateSecretRequest>,
) -> ServiceResult<RotateSecretResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "RotateSecret",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let value = execute(
        service,
        &budget,
        &identity,
        InternalCommand::RotateSecret {
            secret_id: parse_id(request.secret_id.as_option())?,
            expected_active_version_id: parse_id(request.expected_active_version_id.as_option())?,
            value: request
                .secret
                .as_option()
                .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))?
                .value
                .clone(),
        },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_metadata",
        "organization",
    )
    .await?;
    Response::ok(RotateSecretResponse {
        secret_id: opaque(json_id(&value, "secret_id")?).into(),
        version_id: opaque(json_id(&value, "secret_version_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn revoke_secret(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, RevokeSecretRequest>,
) -> ServiceResult<RevokeSecretResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "RevokeSecret",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let secret_id = parse_id(request.secret_id.as_option())?;
    execute(
        service,
        &budget,
        &identity,
        InternalCommand::RevokeSecret { secret_id },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_metadata",
        "organization",
    )
    .await?;
    Response::ok(RevokeSecretResponse {
        secret_id: opaque(secret_id.to_string()).into(),
        state: SecretState::Revoked.into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn set_secret_enabled(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, SetSecretEnabledRequest>,
) -> ServiceResult<SetSecretEnabledResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "SetSecretEnabled",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let secret_id = parse_id(request.secret_id.as_option())?;
    execute(
        service,
        &budget,
        &identity,
        InternalCommand::SetSecretEnabled {
            secret_id,
            enabled: request.enabled,
        },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_metadata",
        "organization",
    )
    .await?;
    Response::ok(SetSecretEnabledResponse {
        secret_id: opaque(secret_id.to_string()).into(),
        state: if request.enabled {
            SecretState::Active
        } else {
            SecretState::Disabled
        }
        .into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn purge_secret(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, PurgeSecretRequest>,
) -> ServiceResult<PurgeSecretResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "PurgeSecret",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let secret_id = parse_id(request.secret_id.as_option())?;
    execute(
        service,
        &budget,
        &identity,
        InternalCommand::PurgeSecret { secret_id },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_metadata",
        "organization",
    )
    .await?;
    Response::ok(PurgeSecretResponse {
        secret_id: opaque(secret_id.to_string()).into(),
        state: SecretState::Purged.into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn grant_secret(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, GrantSecretRequest>,
) -> ServiceResult<GrantSecretResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "GrantSecret",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let value = execute(
        service,
        &budget,
        &identity,
        InternalCommand::GrantSecret {
            secret_id: parse_id(request.secret_id.as_option())?,
            target: secret_target(request.target.as_option())?,
            policy: secret_policy(request.policy.as_option())?,
            expires_at: request
                .expires_at
                .as_option()
                .map(from_timestamp)
                .transpose()?,
        },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_grant",
        "organization",
    )
    .await?;
    Response::ok(GrantSecretResponse {
        grant_id: opaque(json_id(&value, "grant_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

pub(super) async fn accept_secret_import(
    service: &super::SecretRpc,
    ctx: RequestContext,
    request: ServiceRequest<'_, AcceptSecretImportRequest>,
) -> ServiceResult<AcceptSecretImportResponse> {
    let request = request.to_owned_message();
    let identity = mutation(
        &ctx,
        &service.authenticator,
        "AcceptSecretImport",
        &request.context,
    )?;
    let budget = request::RequestBudget::from_transport(&ctx);
    let value = execute(
        service,
        &budget,
        &identity,
        InternalCommand::AcceptSecretImport {
            grant_id: parse_id(request.grant_id.as_option())?,
            target: secret_target(request.target.as_option())?,
            alias: SecretAlias::parse(request.alias).map_err(invalid)?,
        },
    )
    .await?;
    let receipt = receipt(
        &budget,
        &service.receipts,
        identity.idempotency_id,
        identity.user_id,
        "secret_import",
        "organization",
    )
    .await?;
    Response::ok(AcceptSecretImportResponse {
        import_id: opaque(json_id(&value, "import_id")?).into(),
        receipt: receipt.into(),
        ..Default::default()
    })
}

#[cfg(test)]
#[path = "mutations/tests.rs"]
mod tests;
