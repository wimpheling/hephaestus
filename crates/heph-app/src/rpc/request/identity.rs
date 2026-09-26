use crate::rpc::{MediatorAuthenticator, RpcError};
use connectrpc::RequestContext as TransportContext;
use identity_domain::{
    AuthenticatedIdentity, RequestId, actor_idempotency_id, mutation_idempotency_seed,
};
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
use std::str::FromStr;

pub const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

pub fn mutation_identity(
    transport: &TransportContext,
    _authenticator: &MediatorAuthenticator,
    audience: &str,
    context: Option<&RequestContext>,
) -> Result<AuthenticatedIdentity, RpcError> {
    let mut identity = transport
        .extensions()
        .get::<AuthenticatedIdentity>()
        .cloned()
        .ok_or(RpcError::Unauthenticated)?;
    let context = context.ok_or(RpcError::InvalidArgument)?;
    let request_id = mutation_request_id(context)?;
    let idempotency_id = derive_idempotency_id(
        identity.user_id.as_uuid().as_bytes(),
        audience,
        &context.idempotency_key,
    );
    identity.request_id = request_id;
    Ok(identity.with_idempotency_id(idempotency_id))
}

pub fn derive_idempotency_id(
    actor_identity: &[u8],
    audience: &str,
    idempotency_key: &str,
) -> RequestId {
    actor_idempotency_id(
        actor_identity,
        &mutation_idempotency_seed(audience, idempotency_key),
    )
}

pub fn mutation_request_id(context: &RequestContext) -> Result<RequestId, RpcError> {
    if context.idempotency_key.is_empty()
        || context.idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
    {
        return Err(RpcError::InvalidArgument);
    }
    let request_id = required_id(context.request_id.as_option())?;
    RequestId::from_str(&request_id).map_err(|_| RpcError::InvalidArgument)
}

pub fn query_identity(
    transport: &TransportContext,
    _authenticator: &MediatorAuthenticator,
    _audience: &str,
) -> Result<AuthenticatedIdentity, RpcError> {
    transport
        .extensions()
        .get::<AuthenticatedIdentity>()
        .cloned()
        .ok_or(RpcError::Unauthenticated)
}

pub fn verified_mediator_session(
    transport: &TransportContext,
) -> Result<super::super::VerifiedMediatorSession, RpcError> {
    transport
        .extensions()
        .get::<super::super::VerifiedMediatorSession>()
        .copied()
        .ok_or(RpcError::Unauthenticated)
}

pub fn required_id(value: Option<&OpaqueId>) -> Result<String, RpcError> {
    let value = value.ok_or(RpcError::InvalidArgument)?.value.trim();
    uuid::Uuid::parse_str(value).map_err(|_| RpcError::InvalidArgument)?;
    Ok(value.to_owned())
}
