//! Shared authenticated RPC request conversion.

use super::{MediatorAuthenticator, RpcError};
use connectrpc::RequestContext as TransportContext;
use identity_domain::{
    AuthenticatedIdentity, RequestId, actor_idempotency_id, mutation_idempotency_seed,
};
use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
use std::{future::Future, str::FromStr, time::Instant};
use tokio_util::sync::CancellationToken;

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 256;

/// The bounded lifetime shared by one RPC handler and its downstream calls.
///
/// The token is canceled when the handler future drops this owner. Downstream
/// adapters can clone the token and select it alongside their I/O future while
/// [`run_with_budget`] enforces the same absolute transport deadline.
pub(super) struct RequestBudget {
    deadline: Option<Instant>,
    cancellation: CancellationToken,
}

impl RequestBudget {
    pub(super) fn from_transport(transport: &TransportContext) -> Self {
        Self {
            deadline: transport.deadline(),
            cancellation: CancellationToken::new(),
        }
    }

    pub(super) fn cancellation_token(&self) -> CancellationToken {
        self.cancellation.clone()
    }

    #[cfg(test)]
    fn deadline(&self) -> Option<Instant> {
        self.deadline
    }
}

impl Drop for RequestBudget {
    fn drop(&mut self) {
        self.cancellation.cancel();
    }
}

/// Runs one downstream operation until the shared deadline or request
/// cancellation. The operation is dropped when either boundary wins.
pub(super) async fn run_with_budget<T, Operation>(
    budget: &RequestBudget,
    operation: Operation,
) -> Result<T, RpcError>
where
    Operation: Future<Output = T>,
{
    let cancellation = budget.cancellation_token();
    match budget.deadline {
        Some(deadline) => {
            tokio::select! {
                result = tokio::time::timeout_at(deadline.into(), operation) => {
                    result.map_err(|_| RpcError::DeadlineExceeded)
                }
                () = cancellation.cancelled() => Err(RpcError::Canceled),
            }
        }
        None => {
            tokio::select! {
                result = operation => Ok(result),
                () = cancellation.cancelled() => Err(RpcError::Canceled),
            }
        }
    }
}

#[cfg(test)]
fn test_budget(deadline: Option<Instant>) -> RequestBudget {
    RequestBudget {
        deadline,
        cancellation: CancellationToken::new(),
    }
}

pub(super) fn mutation_identity(
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
    _authenticator: &MediatorAuthenticator,
    _audience: &str,
) -> Result<AuthenticatedIdentity, RpcError> {
    transport
        .extensions()
        .get::<AuthenticatedIdentity>()
        .cloned()
        .ok_or(RpcError::Unauthenticated)
}

pub(super) fn verified_mediator_session(
    transport: &TransportContext,
) -> Result<super::VerifiedMediatorSession, RpcError> {
    transport
        .extensions()
        .get::<super::VerifiedMediatorSession>()
        .copied()
        .ok_or(RpcError::Unauthenticated)
}

pub(super) fn required_id(value: Option<&OpaqueId>) -> Result<String, RpcError> {
    let value = value.ok_or(RpcError::InvalidArgument)?.value.trim();
    uuid::Uuid::parse_str(value).map_err(|_| RpcError::InvalidArgument)?;
    Ok(value.to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_IDEMPOTENCY_KEY_BYTES, MediatorAuthenticator, derive_idempotency_id,
        mutation_request_id, run_with_budget, test_budget,
    };
    use rpc_proto::messages::hephaestus::common::v1::{OpaqueId, RequestContext};
    use std::{
        future::Future,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
    };
    use uuid::Uuid;

    struct DropProbe(Arc<AtomicBool>);

    impl Future for DropProbe {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

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
    fn header_only_valid_jwt_cannot_supply_request_identity() {
        use super::mutation_identity;
        use crate::rpc::mediator_signing_key;
        use connectrpc::RequestContext as TransportContext;
        use http::{Extensions, HeaderMap, HeaderValue, header::AUTHORIZATION};
        use identity_domain::{AuthenticatedIdentity, UserId};
        use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
        use serde::Serialize;
        use serde_json::json;
        use time::OffsetDateTime;

        #[derive(Serialize)]
        struct Claims {
            iss: &'static str,
            aud: &'static str,
            sub: String,
            jti: String,
            iat: i64,
            nbf: i64,
            exp: i64,
            sid: String,
        }

        let key = mediator_signing_key(b"request-helper-test-token-with-entropy");
        let audience = "/hephaestus.project.v1.ProjectService/GetProject";
        let now = OffsetDateTime::now_utc().unix_timestamp();
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                iss: "hephaestus-web-mediator",
                aud: audience,
                sub: Uuid::new_v4().to_string(),
                jti: Uuid::new_v4().to_string(),
                iat: now,
                nbf: now,
                exp: now + 30,
                sid: Uuid::new_v4().to_string(),
            },
            &EncodingKey::from_secret(&key),
        )
        .expect("encode valid mediator assertion");
        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).expect("authorization header"),
        );
        let transport = TransportContext::new(headers);
        let authenticator = MediatorAuthenticator::new(&key);
        assert!(
            authenticator
                .authenticate(transport.headers(), audience)
                .is_ok()
        );
        let context = RequestContext {
            request_id: OpaqueId {
                value: Uuid::new_v4().to_string(),
                ..Default::default()
            }
            .into(),
            idempotency_key: String::from("header-only"),
            ..Default::default()
        };
        assert!(matches!(
            mutation_identity(&transport, &authenticator, audience, Some(&context),),
            Err(super::RpcError::Unauthenticated)
        ));
        assert!(matches!(
            super::query_identity(&transport, &authenticator, audience),
            Err(super::RpcError::Unauthenticated)
        ));

        let assertion_id = Uuid::new_v4();
        let body_request_id = Uuid::new_v4();
        let mut extensions = Extensions::new();
        extensions.insert(AuthenticatedIdentity::new(
            UserId::new(),
            "hephaestus-web-mediator",
            "subject",
            json!({"assertion_id": assertion_id}),
            identity_domain::RequestId::from_uuid(assertion_id),
        ));
        let authenticated_transport =
            TransportContext::new(HeaderMap::new()).with_extensions(extensions);
        let body = RequestContext {
            request_id: OpaqueId {
                value: body_request_id.to_string(),
                ..Default::default()
            }
            .into(),
            idempotency_key: String::from("distinct-request-id"),
            ..Default::default()
        };
        let identity = mutation_identity(
            &authenticated_transport,
            &authenticator,
            audience,
            Some(&body),
        )
        .expect("body request context is authoritative");
        assert_eq!(identity.request_id.as_uuid(), body_request_id);
        assert_eq!(
            identity.verified_claims["assertion_id"],
            assertion_id.to_string()
        );
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

    #[tokio::test]
    async fn request_budget_enforces_the_transport_deadline() {
        use connectrpc::RequestContext as TransportContext;
        use http::HeaderMap;
        use std::time::{Duration, Instant};

        let deadline = Instant::now() + Duration::from_millis(1);
        let transport = TransportContext::new(HeaderMap::new()).with_deadline(Some(deadline));
        let budget = super::RequestBudget::from_transport(&transport);
        assert_eq!(budget.deadline(), Some(deadline));

        let dropped = Arc::new(AtomicBool::new(false));
        let result = run_with_budget(&budget, DropProbe(Arc::clone(&dropped))).await;
        assert_eq!(result, Err(super::RpcError::DeadlineExceeded));
        assert!(dropped.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn request_budget_cancels_when_the_request_future_drops() {
        let cancellation = {
            let budget = test_budget(None);
            let cancellation = budget.cancellation_token();
            let task = tokio::spawn(async move {
                let _ = run_with_budget(&budget, std::future::pending::<()>()).await;
            });
            tokio::task::yield_now().await;
            task.abort();
            assert!(task.await.is_err());
            cancellation
        };

        assert!(cancellation.is_cancelled());
    }
}
