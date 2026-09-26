use async_trait::async_trait;
use http::StatusCode;
use std::sync::Arc;
use uuid::Uuid;

use super::{
    runtime::GatewayVmHandler,
    validation::{empty_response, validate_request, validate_ui_request},
};
use crate::{
    Exposure, GatewayInboundSecretResolver, GatewayInvocationOutcome, GatewayInvocationRecorder,
    GatewayMailboxPublisher, GatewayProviderResponse, GatewayRequest, GatewayRequestDispatcher,
    GatewayRouteResolver, UiGatewayAdmissionError, UiGatewayAdmissionProvider, UiGatewayRequest,
    admission_failure_response, prepare_gateway_request,
};

/// Durable audit/session boundary for one invocation.  The control plane owns
const fn ui_admission_dispatch_disposition(
    error: UiGatewayAdmissionError,
) -> UiDispatchDisposition {
    match error {
        UiGatewayAdmissionError::Denied => UiDispatchDisposition::ProviderDenied,
        UiGatewayAdmissionError::NotFound => UiDispatchDisposition::ProviderNotFound,
        UiGatewayAdmissionError::Unavailable => UiDispatchDisposition::ProviderUnavailable,
    }
}

/// Detailed disposition of one UI-origin dispatch.  This is an audit-facing
/// semantic result; HTTP status is deliberately not used to infer admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiDispatchDisposition {
    /// The durable UI authority provider rejected the request.
    ProviderDenied,
    /// The provider found no current declaration/route for the request.
    ProviderNotFound,
    /// The provider could not read current authority or gateway state.
    ProviderUnavailable,
    /// The typed request or selected admission violated the edge contract.
    StructuralInvalid,
    /// Durable `accepted_ui` could not record the invocation.
    AcceptedUiFailure,
    /// The invocation was admitted and its existing terminal outcome is known.
    Admitted {
        /// Existing invocation terminal outcome.
        outcome: GatewayInvocationOutcome,
        /// Whether the terminal outcome was durably persisted.
        completion_persisted: bool,
    },
}

/// Response plus the authoritative UI dispatch disposition.
#[derive(Clone)]
pub struct UiDispatchResult {
    /// Safe response returned by the edge.
    pub response: GatewayProviderResponse,
    /// Typed admission/execution disposition for audit and response policy.
    pub disposition: UiDispatchDisposition,
}

impl UiDispatchResult {
    fn into_response(self) -> GatewayProviderResponse {
        self.response
    }
}

/// A synchronous dispatcher with deliberate response fallbacks.
pub struct GatewayDispatcher<R, H, I> {
    pub(crate) resolver: R,
    pub(crate) handler: H,
    pub(crate) recorder: I,
    pub(crate) inbound_secrets: Option<Arc<dyn GatewayInboundSecretResolver>>,
    pub(crate) mailbox_publisher: Option<Arc<dyn GatewayMailboxPublisher>>,
}

impl<R, H, I> GatewayDispatcher<R, H, I> {
    /// Creates a dispatcher from explicit authority, VM, and audit ports.
    #[must_use]
    pub const fn new(resolver: R, handler: H, recorder: I) -> Self {
        Self {
            resolver,
            handler,
            recorder,
            inbound_secrets: None,
            mailbox_publisher: None,
        }
    }

    /// Adds the host-only exact-lease resolver used for inbound placeholders.
    #[must_use]
    pub fn with_inbound_secret_resolver(
        mut self,
        resolver: Arc<dyn GatewayInboundSecretResolver>,
    ) -> Self {
        self.inbound_secrets = Some(resolver);
        self
    }

    /// Adds the exact-bound mailbox publisher used by gateway releases that
    /// return a publication candidate.
    #[must_use]
    pub fn with_mailbox_publisher(mut self, publisher: Arc<dyn GatewayMailboxPublisher>) -> Self {
        self.mailbox_publisher = Some(publisher);
        self
    }
}

#[async_trait]
impl<R, H, I> GatewayRequestDispatcher for GatewayDispatcher<R, H, I>
where
    R: GatewayRouteResolver,
    H: GatewayVmHandler,
    I: GatewayInvocationRecorder,
{
    async fn dispatch(&self, request: GatewayRequest) -> GatewayProviderResponse {
        let fallback = |status| GatewayProviderResponse {
            response: empty_response(status),
            invocation_id: Uuid::nil(),
        };
        let Some(route) = (match self.resolver.resolve(&request.path_and_query).await {
            Ok(route) => route,
            Err(_) => return fallback(StatusCode::SERVICE_UNAVAILABLE),
        }) else {
            return fallback(StatusCode::NOT_FOUND);
        };
        if route.exposure != Exposure::Public {
            return fallback(StatusCode::NOT_FOUND);
        }
        if let Err(error) = validate_request(&route, &request) {
            let _ = error;
            return fallback(StatusCode::BAD_REQUEST);
        }
        let Ok(invocation_id) = self
            .recorder
            .accepted(&route, request.trusted.request_id)
            .await
        else {
            return fallback(StatusCode::SERVICE_UNAVAILABLE);
        };
        self.dispatch_admitted(route, request, invocation_id, false)
            .await
            .into_response()
    }
}

impl<R, H, I> GatewayDispatcher<R, H, I> {
    /// Dispatches a request from the trusted UI origin after durable UI
    /// admission. Public dispatch remains restricted to `Exposure::Public`.
    pub async fn dispatch_ui<A>(
        &self,
        request: UiGatewayRequest,
        authority: &A,
    ) -> GatewayProviderResponse
    where
        R: GatewayRouteResolver,
        H: GatewayVmHandler,
        I: GatewayInvocationRecorder,
        A: UiGatewayAdmissionProvider + ?Sized,
    {
        self.dispatch_ui_detailed(request, authority)
            .await
            .into_response()
    }

    /// Dispatches one trusted UI request and retains the authoritative
    /// admission/execution disposition for audit. The response-only
    /// [`Self::dispatch_ui`] wrapper remains the compatibility surface.
    pub async fn dispatch_ui_detailed<A>(
        &self,
        request: UiGatewayRequest,
        authority: &A,
    ) -> UiDispatchResult
    where
        R: GatewayRouteResolver,
        H: GatewayVmHandler,
        I: GatewayInvocationRecorder,
        A: UiGatewayAdmissionProvider + ?Sized,
    {
        let admission = match authority.admit(&request).await {
            Ok(admission) => admission,
            Err(error) => {
                return UiDispatchResult {
                    response: GatewayProviderResponse {
                        response: admission_failure_response(error),
                        invocation_id: Uuid::nil(),
                    },
                    disposition: ui_admission_dispatch_disposition(error),
                };
            }
        };
        let request_authority = request.authority.clone();
        let Ok(request) = prepare_gateway_request(&request, &admission) else {
            return UiDispatchResult {
                response: GatewayProviderResponse {
                    response: empty_response(StatusCode::BAD_REQUEST),
                    invocation_id: Uuid::nil(),
                },
                disposition: UiDispatchDisposition::StructuralInvalid,
            };
        };
        if validate_ui_request(&admission.route, &request).is_err() {
            return UiDispatchResult {
                response: GatewayProviderResponse {
                    response: empty_response(StatusCode::BAD_REQUEST),
                    invocation_id: Uuid::nil(),
                },
                disposition: UiDispatchDisposition::StructuralInvalid,
            };
        }
        let Ok(invocation_id) = self
            .recorder
            .accepted_ui(
                &admission.route,
                &request_authority,
                request.trusted.request_id,
            )
            .await
        else {
            return UiDispatchResult {
                response: GatewayProviderResponse {
                    response: empty_response(StatusCode::SERVICE_UNAVAILABLE),
                    invocation_id: Uuid::nil(),
                },
                disposition: UiDispatchDisposition::AcceptedUiFailure,
            };
        };
        self.dispatch_admitted(admission.route, request, invocation_id, true)
            .await
    }
}
