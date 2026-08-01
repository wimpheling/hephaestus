//! Shared authenticated RPC request conversion.

use super::{MediatorAuthenticator, RpcError};
use connectrpc::RequestContext as TransportContext;
use identity_domain::{
    AuthenticatedIdentity, RequestId, actor_idempotency_id, mutation_idempotency_seed,
};
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
use serde_json::json;
use std::str::FromStr;

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

pub(super) fn mutation_identity(
    transport: &TransportContext,
    authenticator: &MediatorAuthenticator,
    audience: &str,
    context: Option<&RequestContext>,
) -> Result<AuthenticatedIdentity, RpcError> {
    let principal = transport
        .extensions()
        .get::<AuthenticatedIdentity>()
        .cloned()
        .map(|identity| super::MediatorPrincipal {
            user_id: identity.user_id,
            assertion_id: identity.request_id.as_uuid(),
        });
    let principal = match principal {
        Some(principal) => principal,
        None => authenticator
            .authenticate(transport.headers(), audience)
            .map_err(|_| RpcError::Unauthenticated)?,
    };
    let context = context.ok_or(RpcError::InvalidArgument)?;
    let request_id = mutation_request_id(context)?;
    let idempotency_id = derive_idempotency_id(
        principal.user_id.as_uuid().as_bytes(),
        audience,
        &context.idempotency_key,
    );
    Ok(AuthenticatedIdentity::new(
        principal.user_id,
        "hephaestus-web-mediator",
        principal.user_id.to_string(),
        json!({"mediator": "phoenix", "assertion_id": principal.assertion_id}),
        request_id,
    )
    .with_idempotency_id(idempotency_id))
}

pub(super) fn derive_idempotency_id(
    actor_identity: &[u8],
    audience: &str,
    idempotency_key: &str,
) -> RequestId {
    actor_idempotency_id(
        actor_identity,
        &mutation_idempotency_seed(audience, idempotency_key),
    )
}

fn mutation_request_id(context: &RequestContext) -> Result<RequestId, RpcError> {
    if context.idempotency_key.is_empty()
        || context.idempotency_key.len() > MAX_IDEMPOTENCY_KEY_BYTES
    {
        return Err(RpcError::InvalidArgument);
    }
    let request_id = required_id(context.request_id.as_option())?;
    RequestId::from_str(&request_id).map_err(|_| RpcError::InvalidArgument)
}

pub(super) fn query_identity(
    transport: &TransportContext,
    authenticator: &MediatorAuthenticator,
    audience: &str,
) -> Result<AuthenticatedIdentity, RpcError> {
    if let Some(identity) = transport.extensions().get::<AuthenticatedIdentity>() {
        return Ok(identity.clone());
    }
    let principal = authenticator
        .authenticate(transport.headers(), audience)
        .map_err(|_| RpcError::Unauthenticated)?;
    Ok(AuthenticatedIdentity::new(
        principal.user_id,
        "hephaestus-web-mediator",
        principal.user_id.to_string(),
        json!({"mediator": "phoenix", "assertion_id": principal.assertion_id}),
        RequestId::from_uuid(principal.assertion_id),
    ))
}

pub(super) fn required_id(value: Option<&OpaqueId>) -> Result<String, RpcError> {
    let value = value.ok_or(RpcError::InvalidArgument)?.value.trim();
    uuid::Uuid::parse_str(value).map_err(|_| RpcError::InvalidArgument)?;
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{MAX_IDEMPOTENCY_KEY_BYTES, derive_idempotency_id, mutation_request_id};
    use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
    use uuid::Uuid;

    #[test]
    fn mutation_context_requires_bounded_idempotency_and_uuid_request_id() {
        let valid = RequestContext {
            request_id: OpaqueId {
                value: Uuid::new_v4().to_string(),
                ..Default::default()
            }
            .into(),
            idempotency_key: String::from("browser-action"),
            ..Default::default()
        };
        assert!(mutation_request_id(&valid).is_ok());

        let mut invalid = valid;
        invalid.idempotency_key.clear();
        assert!(mutation_request_id(&invalid).is_err());
        invalid.idempotency_key = "x".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1);
        assert!(mutation_request_id(&invalid).is_err());
        invalid.idempotency_key = String::from("browser-action");
        invalid.request_id.get_or_insert_default().value = String::from("not-a-uuid");
        assert!(mutation_request_id(&invalid).is_err());
    }

    #[test]
    fn idempotency_identity_is_stable_and_actor_method_scoped() {
        let actor = Uuid::new_v4();
        let other_actor = Uuid::new_v4();
        let first = derive_idempotency_id(actor.as_bytes(), "/service.v1/First", "retry-key");
        assert_eq!(
            first,
            derive_idempotency_id(actor.as_bytes(), "/service.v1/First", "retry-key")
        );
        assert_ne!(
            first,
            derive_idempotency_id(other_actor.as_bytes(), "/service.v1/First", "retry-key")
        );
        assert_ne!(
            first,
            derive_idempotency_id(actor.as_bytes(), "/service.v1/Second", "retry-key")
        );
        assert_ne!(
            first,
            derive_idempotency_id(actor.as_bytes(), "/service.v1/First", "other-key")
        );
        assert_eq!(first.as_uuid().get_version_num(), 8);
    }
}
