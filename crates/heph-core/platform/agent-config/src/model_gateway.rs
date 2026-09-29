//! Repository gateway configuration declarations.
use capability_domain::CapabilitySlotKey;
use gateway_domain::{
    Exposure, GatewayDeclaration, GatewayMailboxPublicationSlot, GatewayName, GatewayServiceConfig,
    HttpMethod, RouteIntent, RoutePath, ServiceLogCaptureMode, ServiceProbePath,
};
use serde::{Deserialize, Serialize};

/// Repository-owned gateway declarations read from `heph.gateways.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGatewaysConfig {
    /// Manifest schema version.
    pub version: u32,
    /// Gateway workloads declared by this repository.
    #[serde(default)]
    pub gateways: Vec<RepositoryGatewayConfig>,
}

/// One repository-local gateway declaration.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGatewayConfig {
    /// Stable repository-scoped gateway name.
    pub name: String,
    /// Exact released agent key whose immutable runtime contract handles HTTP.
    pub agent_name: String,
    /// Versioned handler contract (`http.v1` or `http.service.v1`).
    pub handler_contract: String,
    /// Required only for the `http.service.v1` contract. Runtime wiring is
    /// intentionally deferred until the service transport is integrated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<RepositoryGatewayServiceConfig>,
    /// Whether the future provider exposes the route publicly or through
    /// Hephaestus authentication.
    pub exposure: Exposure,
    /// Bounded route requests owned by this declaration.
    pub routes: Vec<RepositoryGatewayRouteConfig>,
    /// Typed, non-secret declaration parameters.
    #[serde(default)]
    pub parameters: toml::Table,
    /// Symbolic brokered-secret slots only, never tenant secret values.
    #[serde(default)]
    pub secret_slots: Vec<String>,
    /// Required, fixed-shape mailbox-publication requirements. Each slot can
    /// only bind one explicit mailbox and producer identity at installation.
    #[serde(default)]
    pub mailbox_publication_slots: Vec<RepositoryGatewayMailboxPublicationSlot>,
}

/// Repository-declared settings for a long-lived HTTP gateway service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGatewayServiceConfig {
    /// Exact TCP port on `127.0.0.1` inside the guest.
    pub loopback_port: u16,
    /// Origin path used to establish service readiness.
    pub readiness_path: String,
    /// Origin path used for ongoing service health checks.
    pub health_path: String,
    /// Optional project-scoped application log capture policy.
    #[serde(default, skip_serializing_if = "ServiceLogCaptureMode::is_disabled")]
    pub log_capture_mode: ServiceLogCaptureMode,
}

impl RepositoryGatewayServiceConfig {
    fn to_declaration(&self) -> Result<GatewayServiceConfig, gateway_domain::GatewayError> {
        Ok(GatewayServiceConfig::new(
            self.loopback_port,
            ServiceProbePath::parse(self.readiness_path.clone())?,
            ServiceProbePath::parse(self.health_path.clone())?,
        )?
        .with_log_capture_mode(self.log_capture_mode))
    }
}

/// One repository-declared required mailbox publication slot for a gateway.
///
/// Its capability shape is fixed by the platform: `mailbox` + `publish`, with
/// no optional operations and a required binding. The manifest therefore
/// accepts no resource kind or operation fields that could broaden authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGatewayMailboxPublicationSlot {
    /// Stable release-owned capability slot key.
    pub key: String,
    /// Human-readable non-secret reason for this publication path.
    pub purpose: String,
}

impl RepositoryGatewayMailboxPublicationSlot {
    /// Converts the slot into the shared gateway mailbox contract.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed keys or purposes.
    pub fn to_declaration(
        &self,
    ) -> Result<GatewayMailboxPublicationSlot, gateway_domain::GatewayError> {
        let key = CapabilitySlotKey::parse(self.key.clone())
            .map_err(|_| gateway_domain::GatewayError::InvalidMailboxPublicationSlot)?;
        GatewayMailboxPublicationSlot::new(key, self.purpose.clone())
    }
}

impl RepositoryGatewayConfig {
    /// Converts the repository source form into the shared gateway contract.
    ///
    /// # Errors
    ///
    /// Returns a gateway-domain error when the declaration exceeds the
    /// bounded HTTP gateway contract.
    pub fn to_declaration(&self) -> Result<GatewayDeclaration, gateway_domain::GatewayError> {
        let routes = self
            .routes
            .iter()
            .map(RepositoryGatewayRouteConfig::to_intent)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(GatewayDeclaration {
            name: GatewayName::parse(self.name.clone())?,
            agent_name: GatewayName::parse(self.agent_name.clone())?
                .as_str()
                .to_owned(),
            handler_contract: self.handler_contract.clone(),
            service: self
                .service
                .as_ref()
                .map(RepositoryGatewayServiceConfig::to_declaration)
                .transpose()?,
            exposure: self.exposure,
            routes,
            parameters: serde_json::to_value(&self.parameters)
                .map_err(|_| gateway_domain::GatewayError::InvalidDeclaration)?,
            secret_slots: self.secret_slots.clone(),
            mailbox_publication_slots: self
                .mailbox_publication_slots
                .iter()
                .map(RepositoryGatewayMailboxPublicationSlot::to_declaration)
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

/// One bounded route in a repository gateway declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositoryGatewayRouteConfig {
    /// Canonical route path.
    pub path: String,
    /// HTTP methods accepted on this route.
    pub methods: Vec<HttpMethod>,
}

impl RepositoryGatewayRouteConfig {
    fn to_intent(&self) -> Result<RouteIntent, gateway_domain::GatewayError> {
        RouteIntent::new(RoutePath::parse(self.path.clone())?, self.methods.clone())
    }
}
