use super::caddy_config::LocalCaddyConfigurationTemplate;
use crate::{
    GatewayConfigRevision, GatewayDesiredConfiguration, GatewayEdgeError, GatewayProvider,
    GatewayProviderResponse, GatewayRequest, GatewayRequestDispatcher, GatewayRouteBinding,
};
use async_trait::async_trait;
use std::collections::BTreeSet;
use std::sync::Arc;

/// Private Caddy administration port.  Implementations must not be exposed to
/// gateway VMs or public request paths.
#[async_trait]
pub trait CaddyAdministration: Send + Sync {
    /// Atomically loads a complete derived Caddy configuration.
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError>;
}

#[async_trait]
impl<T> CaddyAdministration for Arc<T>
where
    T: CaddyAdministration + ?Sized,
{
    async fn load(&self, configuration: Vec<u8>) -> Result<(), GatewayEdgeError> {
        (**self).load(configuration).await
    }
}

/// Minimal Caddy adapter: it derives `/gateway/` routes only and forwards
/// private normalized HTTP to a stable dispatcher.
pub struct LocalCaddyGatewayProvider<A, D> {
    administration: A,
    dispatcher: D,
    dispatcher_upstream: String,
    template: Option<LocalCaddyConfigurationTemplate>,
    observed: tokio::sync::RwLock<Option<GatewayConfigRevision>>,
}

impl<A, D> LocalCaddyGatewayProvider<A, D> {
    /// Constructs an adapter with no observed configuration after a restart.
    #[must_use]
    pub fn new(administration: A, dispatcher: D) -> Self {
        Self {
            administration,
            dispatcher,
            dispatcher_upstream: String::from("gateway-dispatcher.private"),
            template: None,
            observed: tokio::sync::RwLock::const_new(None),
        }
    }

    /// Sets the private loopback upstream that the derived Caddy routes use.
    ///
    /// The caller owns binding and protecting this endpoint.  The default is
    /// retained for isolated adapter tests only; production composition uses
    /// an explicit loopback address.
    #[must_use]
    pub fn with_dispatcher_upstream(mut self, dispatcher_upstream: String) -> Self {
        self.dispatcher_upstream = dispatcher_upstream;
        self
    }

    /// Supplies the complete operator-owned shared-Caddy configuration and
    /// its one dedicated gateway subroute slot.
    #[must_use]
    pub fn with_configuration_template(
        mut self,
        template: LocalCaddyConfigurationTemplate,
    ) -> Self {
        self.template = Some(template);
        self
    }

    /// Returns the last successfully applied revision.  `None` forces safe
    /// deterministic reconciliation after a process restart.
    pub async fn observed_revision(&self) -> Option<GatewayConfigRevision>
    where
        A: Sync,
        D: Sync,
    {
        *self.observed.read().await
    }
}

#[async_trait]
impl<A, D> GatewayProvider for LocalCaddyGatewayProvider<A, D>
where
    A: CaddyAdministration,
    D: GatewayRequestDispatcher,
{
    async fn reconcile(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        for route in &desired.routes {
            route.validate()?;
        }
        ensure_unique_routes(&desired.routes)?;
        if self.observed_revision().await == Some(desired.revision) {
            return Ok(desired.revision);
        }
        let template = self
            .template
            .as_ref()
            .ok_or(GatewayEdgeError::Unavailable)?;
        let config = template.render(desired, &self.dispatcher_upstream)?;
        self.administration.load(config).await?;
        *self.observed.write().await = Some(desired.revision);
        Ok(desired.revision)
    }

    async fn recover(
        &self,
        desired: &GatewayDesiredConfiguration,
    ) -> Result<GatewayConfigRevision, GatewayEdgeError> {
        // Caddy does not retain an observation token across a process restart.
        // Always reload the complete authoritative template here, even when
        // this daemon still remembers the same desired revision.
        for route in &desired.routes {
            route.validate()?;
        }
        ensure_unique_routes(&desired.routes)?;
        let template = self
            .template
            .as_ref()
            .ok_or(GatewayEdgeError::Unavailable)?;
        let config = template.render(desired, &self.dispatcher_upstream)?;
        self.administration.load(config).await?;
        *self.observed.write().await = Some(desired.revision);
        Ok(desired.revision)
    }

    async fn forward(&self, request: GatewayRequest) -> GatewayProviderResponse {
        self.dispatcher.dispatch(request).await
    }
}

fn ensure_unique_routes(routes: &[GatewayRouteBinding]) -> Result<(), GatewayEdgeError> {
    let mut prefixes = BTreeSet::new();
    for route in routes {
        if !prefixes.insert(route.path_prefix.as_str()) {
            return Err(GatewayEdgeError::InvalidRoute(
                "duplicate active gateway path",
            ));
        }
    }
    Ok(())
}
