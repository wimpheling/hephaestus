use http::StatusCode;
use subtle::ConstantTimeEq;
use tokio::time::timeout;
use uuid::Uuid;

use super::{
    dispatcher::{GatewayDispatcher, UiDispatchDisposition, UiDispatchResult},
    runtime::GatewayVmHandler,
    validation::{empty_response, validate_response},
};
use crate::{
    GatewayEdgeError, GatewayInvocationOutcome, GatewayInvocationRecorder, GatewayProviderResponse,
    GatewayRequest, GatewayRouteBinding, validate_ui_response,
};

impl<R: Sync, H: Sync, I: Sync> GatewayDispatcher<R, H, I> {
    pub(crate) async fn dispatch_admitted(
        &self,
        route: GatewayRouteBinding,
        request: GatewayRequest,
        invocation_id: Uuid,
        ui_response_policy: bool,
    ) -> UiDispatchResult
    where
        H: GatewayVmHandler,
        I: GatewayInvocationRecorder,
    {
        let Ok(request) = self
            .rewrite_inbound_secrets(invocation_id, &route, request)
            .await
        else {
            let completion_persisted = self
                .recorder
                .completed(invocation_id, GatewayInvocationOutcome::Rejected)
                .await
                .is_ok();
            // Missing, repeated, mismatched, revoked, and expired values
            // share a bounded authentication failure without exposing which
            // credential check failed. Unknown routes remain 404.
            return UiDispatchResult {
                response: GatewayProviderResponse {
                    response: empty_response(StatusCode::UNAUTHORIZED),
                    invocation_id: if ui_response_policy {
                        invocation_id
                    } else {
                        Uuid::nil()
                    },
                },
                disposition: UiDispatchDisposition::Admitted {
                    outcome: GatewayInvocationOutcome::Rejected,
                    completion_persisted,
                },
            };
        };
        let result = timeout(
            route.limits.execution_timeout,
            self.handler.invoke(&route, invocation_id, request),
        )
        .await;
        let (response, outcome) = match result {
            Ok(Ok(mut response))
                if validate_response(&route, &response).is_ok()
                    && (!ui_response_policy || validate_ui_response(&response).is_ok()) =>
            {
                if let Some(publication) = response.mailbox_publication.take() {
                    let publication_result = match &self.mailbox_publisher {
                        Some(publisher) => publisher.publish(invocation_id, publication).await,
                        None => Err(GatewayEdgeError::HandlerUnavailable),
                    };
                    if publication_result.is_err() {
                        (
                            empty_response(StatusCode::BAD_GATEWAY),
                            GatewayInvocationOutcome::Failed,
                        )
                    } else {
                        (response, GatewayInvocationOutcome::Completed)
                    }
                } else {
                    (response, GatewayInvocationOutcome::Completed)
                }
            }
            Ok(Ok(_) | Err(_)) => (
                empty_response(StatusCode::BAD_GATEWAY),
                GatewayInvocationOutcome::Failed,
            ),
            Err(_) => (
                empty_response(StatusCode::GATEWAY_TIMEOUT),
                GatewayInvocationOutcome::TimedOut,
            ),
        };
        // Do not acknowledge a publication (or any handler result) when its
        // terminal invocation disposition was not durably recorded. The
        // caller can safely retry: mailbox acceptance is deduplicated by the
        // bound producer/key, while this invocation remains visibly pending
        // for operator recovery rather than being silently forgotten.
        let completion_persisted = self
            .recorder
            .completed(invocation_id, outcome)
            .await
            .is_ok();
        UiDispatchResult {
            response: GatewayProviderResponse {
                response: if completion_persisted {
                    response
                } else {
                    empty_response(StatusCode::SERVICE_UNAVAILABLE)
                },
                invocation_id,
            },
            disposition: UiDispatchDisposition::Admitted {
                outcome,
                completion_persisted,
            },
        }
    }

    async fn rewrite_inbound_secrets(
        &self,
        invocation_id: Uuid,
        route: &GatewayRouteBinding,
        mut request: GatewayRequest,
    ) -> Result<GatewayRequest, GatewayEdgeError> {
        let Some(resolver) = &self.inbound_secrets else {
            return Ok(request);
        };
        let rules = resolver
            .rules_for_invocation(invocation_id, route.route_id, route.gateway_revision_id)
            .await
            .map_err(|_| GatewayEdgeError::Unavailable)?;
        for rule in rules {
            let values = request.headers.get_all(&rule.header);
            let mut values = values.iter();
            let Some(value) = values.next() else {
                return Err(GatewayEdgeError::SecretRejected);
            };
            if values.next().is_some() || !bool::from(value.as_bytes().ct_eq(&rule.expected)) {
                return Err(GatewayEdgeError::SecretRejected);
            }
            request
                .headers
                .insert(rule.header.clone(), rule.placeholder.clone());
        }
        Ok(request)
    }
}
