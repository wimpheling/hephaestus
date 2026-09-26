use crate::rpc::{
    MediatorAuthenticator, MutationReceipts, RpcError, into_connect_error, mutation_receipt,
    request,
};
use connectrpc::{ConnectError, RequestContext};
use gateway_postgres::GatewayPage;
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, PageRequest};
use std::str::FromStr;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 50;
const MAX_PAGE_SIZE: u32 = 100;

pub(super) fn query(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
) -> Result<identity_domain::AuthenticatedIdentity, ConnectError> {
    request::query_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.gateway.v1.GatewayService/{method}"),
    )
    .map_err(into_connect_error)
}

pub(super) fn id(value: Option<&OpaqueId>) -> Result<Uuid, ConnectError> {
    value
        .ok_or_else(|| into_connect_error(RpcError::InvalidArgument))
        .and_then(|value| {
            Uuid::from_str(&value.value).map_err(|_| into_connect_error(RpcError::InvalidArgument))
        })
}

pub(super) fn page(value: Option<&PageRequest>) -> Result<GatewayPage, ConnectError> {
    let size = value.map_or(DEFAULT_PAGE_SIZE, |value| {
        if value.page_size == 0 {
            DEFAULT_PAGE_SIZE
        } else {
            value.page_size
        }
    });
    if size > MAX_PAGE_SIZE {
        return Err(into_connect_error(RpcError::InvalidArgument));
    }
    let after = value
        .filter(|value| !value.page_token.is_empty())
        .map(|value| Uuid::from_str(&value.page_token))
        .transpose()
        .map_err(|_| into_connect_error(RpcError::InvalidArgument))?;
    Ok(GatewayPage {
        limit: i64::from(size),
        after,
    })
}

pub(super) fn mutation(
    ctx: &RequestContext,
    authenticator: &MediatorAuthenticator,
    method: &str,
    context: Option<&rpc_proto::messages::hephaestus::common::v1::RequestContext>,
) -> Result<identity_domain::AuthenticatedIdentity, ConnectError> {
    request::mutation_identity(
        ctx,
        authenticator,
        &format!("/hephaestus.gateway.v1.GatewayService/{method}"),
        context,
    )
    .map_err(into_connect_error)
}

pub(super) async fn receipt(
    receipts: &MutationReceipts,
    identity: &identity_domain::AuthenticatedIdentity,
) -> Result<rpc_proto::messages::hephaestus::common::v1::MutationReceipt, ConnectError> {
    mutation_receipt(
        receipts,
        identity.idempotency_id,
        identity.user_id,
        "gateway",
        "project",
    )
    .await
}
