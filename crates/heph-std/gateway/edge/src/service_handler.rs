//! Routes accepted gateway invocations to stateless handlers or warm services.

use async_trait::async_trait;
use tokio::time::{self, Instant};
use uuid::Uuid;

use crate::{
    GatewayEdgeError, GatewayExecutionTarget, GatewayExecutionTargetResolver, GatewayRequest,
    GatewayResponse, GatewayRouteBinding, GatewayServiceOwner, GatewayServiceRegistry,
    GatewayVmHandler, ServiceHttpPolicy,
};

/// Dispatches an already accepted invocation to its immutable execution target.
pub struct GatewayServiceHandler<R, H> {
    resolver: R,
    stateless: H,
    registry: GatewayServiceRegistry,
    owner: GatewayServiceOwner,
}

impl<R, H> GatewayServiceHandler<R, H> {
    /// Creates a dispatcher with one local service owner and warm-instance
    /// registry.
    ///
    /// # Errors
    ///
    /// Returns an invalid-route error when the configured owner is malformed.
    pub fn new(
        resolver: R,
        stateless: H,
        registry: GatewayServiceRegistry,
        owner: GatewayServiceOwner,
    ) -> Result<Self, GatewayEdgeError> {
        owner
            .validate()
            .map_err(|_| GatewayEdgeError::InvalidRoute("invalid service owner"))?;
        Ok(Self {
            resolver,
            stateless,
            registry,
            owner,
        })
    }
}

#[async_trait]
impl<R, H> GatewayVmHandler for GatewayServiceHandler<R, H>
where
    R: GatewayExecutionTargetResolver,
    H: GatewayVmHandler,
{
    async fn invoke(
        &self,
        route: &GatewayRouteBinding,
        invocation_id: Uuid,
        request: GatewayRequest,
    ) -> Result<GatewayResponse, GatewayEdgeError> {
        let route_deadline = Instant::now()
            .checked_add(route.limits.execution_timeout)
            .ok_or(GatewayEdgeError::HandlerUnavailable)?;
        let target = time::timeout_at(
            route_deadline,
            self.resolver.resolve_execution_target(
                invocation_id,
                route.route_id,
                route.gateway_revision_id,
                &self.owner,
            ),
        )
        .await
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
        .map_err(|_| GatewayEdgeError::HandlerUnavailable)?;

        match target {
            GatewayExecutionTarget::Stateless => {
                let response = time::timeout_at(
                    route_deadline,
                    self.stateless.invoke(route, invocation_id, request),
                )
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)??;
                Ok(response)
            }
            GatewayExecutionTarget::Service(budget) => {
                let authority_deadline = Instant::now()
                    .checked_add(budget.remaining)
                    .ok_or(GatewayEdgeError::HandlerUnavailable)?
                    .min(route_deadline);
                let exchange_timeout = authority_deadline.saturating_duration_since(Instant::now());
                if exchange_timeout.is_zero() {
                    return Err(GatewayEdgeError::HandlerUnavailable);
                }
                let policy = ServiceHttpPolicy::from_gateway_limits(route.limits);
                let policy = ServiceHttpPolicy {
                    exchange_timeout,
                    ..policy
                };
                time::timeout_at(
                    authority_deadline,
                    self.registry.exchange(budget.instance, request, policy),
                )
                .await
                .map_err(|_| GatewayEdgeError::HandlerUnavailable)?
            }
        }
    }
}

#[cfg(test)]
#[path = "service_handler/tests.rs"]
mod tests;
