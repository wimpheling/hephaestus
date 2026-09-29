//! Gateway installation declarations and validation errors.

use capability_domain::{CapabilityOperation, CapabilityResourceKind, CapabilitySlotKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use thiserror::Error;

use super::{
    Exposure, GatewayName, HTTP_HANDLER_CONTRACT_V1, HTTP_SERVICE_HANDLER_CONTRACT_V1,
    MAX_MAILBOX_PUBLICATION_SLOTS, MAX_ROUTES, RouteIntent, ServiceProbePath,
};

/// Whether a long-lived service opts into project-scoped application logs.
///
/// Lifecycle diagnostics remain content-free, and this setting does not claim
/// to redact application-owned output.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ServiceLogCaptureMode {
    /// Do not capture application stdout or stderr.
    #[default]
    Disabled,
    /// Opt into the bounded project-scoped application log stream.
    Application,
}

impl ServiceLogCaptureMode {
    /// Returns the stable storage and manifest spelling of this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Disabled => "disabled",
            Self::Application => "application",
        }
    }

    /// Parses the closed storage and manifest spelling of this mode.
    #[must_use]
    pub fn from_name(value: &str) -> Option<Self> {
        match value {
            "disabled" => Some(Self::Disabled),
            "application" => Some(Self::Application),
            _ => None,
        }
    }

    /// Returns whether application log capture is disabled.
    #[must_use]
    pub const fn is_disabled(&self) -> bool {
        matches!(self, Self::Disabled)
    }
}

/// Typed configuration for the initial long-lived HTTP service contract.
///
/// The port is guest-loopback only. Readiness and health paths use the same
/// strict origin-path grammar of [`ServiceProbePath`]. Platform-selected
/// timeout and concurrency limits are deliberately not release-controlled
/// settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GatewayServiceConfig {
    /// Exact TCP port on `127.0.0.1` inside the guest.
    pub loopback_port: u16,
    /// Origin path used to establish service readiness.
    pub readiness_path: ServiceProbePath,
    /// Origin path used for ongoing service health checks.
    pub health_path: ServiceProbePath,
    /// Optional project-scoped application log capture policy.
    #[serde(default, skip_serializing_if = "ServiceLogCaptureMode::is_disabled")]
    pub log_capture_mode: ServiceLogCaptureMode,
}

impl GatewayServiceConfig {
    /// Creates and validates one service configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the loopback port is privileged or either path is
    /// outside the canonical gateway path grammar.
    pub fn new(
        loopback_port: u16,
        readiness_path: ServiceProbePath,
        health_path: ServiceProbePath,
    ) -> Result<Self, GatewayError> {
        let configuration = Self {
            loopback_port,
            readiness_path,
            health_path,
            log_capture_mode: ServiceLogCaptureMode::Disabled,
        };
        configuration.validate()?;
        Ok(configuration)
    }

    /// Selects the declaration's application log capture policy.
    #[must_use]
    pub const fn with_log_capture_mode(mut self, mode: ServiceLogCaptureMode) -> Self {
        self.log_capture_mode = mode;
        self
    }

    /// Validates an already constructed service configuration.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayError::InvalidServicePort`] for a privileged port.
    pub const fn validate(&self) -> Result<(), GatewayError> {
        if self.loopback_port < 1024 {
            return Err(GatewayError::InvalidServicePort);
        }
        Ok(())
    }
}

/// Immutable source declaration used to install one gateway revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayDeclaration {
    /// Stable name.
    pub name: GatewayName,
    /// Exact release-agent key selected as the immutable handler target.
    pub agent_name: String,
    /// Must equal [`HTTP_HANDLER_CONTRACT_V1`] or
    /// [`HTTP_SERVICE_HANDLER_CONTRACT_V1`].
    pub handler_contract: String,
    /// Required only for [`HTTP_SERVICE_HANDLER_CONTRACT_V1`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<GatewayServiceConfig>,
    /// Exposure policy.
    pub exposure: Exposure,
    /// Bounded route intent.
    pub routes: Vec<RouteIntent>,
    /// Typed non-secret parameters represented as canonical JSON.
    pub parameters: serde_json::Value,
    /// Symbolic secret slot names only.
    pub secret_slots: Vec<String>,
    /// Required, fixed-shape authority to publish to explicitly bound agent
    /// mailboxes. These are symbolic release requirements; a gateway never
    /// chooses a mailbox or producer identity at invocation time.
    pub mailbox_publication_slots: Vec<GatewayMailboxPublicationSlot>,
}

/// One symbolic, required mailbox-publication requirement of a gateway revision.
///
/// The shape is intentionally closed: every declared slot requires a mailbox
/// resource and the `publish` operation. Binding a concrete mailbox and its
/// stable producer identity remains a control-plane action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayMailboxPublicationSlot {
    /// Stable release-owned slot key.
    pub key: CapabilitySlotKey,
    /// Human-readable, non-secret reason the gateway publishes to this mailbox.
    pub purpose: String,
}

impl GatewayMailboxPublicationSlot {
    /// Creates one required fixed-shape mailbox publication slot.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayError::InvalidMailboxPublicationSlot`] if the purpose
    /// is empty or exceeds the bounded declaration size.
    pub fn new(key: CapabilitySlotKey, purpose: impl Into<String>) -> Result<Self, GatewayError> {
        let purpose = purpose.into();
        if purpose.trim().is_empty() || purpose.len() > 512 {
            return Err(GatewayError::InvalidMailboxPublicationSlot);
        }
        Ok(Self { key, purpose })
    }

    /// Returns the only resource category this slot can bind.
    #[must_use]
    pub const fn resource_kind(&self) -> CapabilityResourceKind {
        CapabilityResourceKind::Mailbox
    }

    /// Returns the only operation every binding must grant.
    #[must_use]
    pub const fn required_operation(&self) -> CapabilityOperation {
        CapabilityOperation::Publish
    }

    /// Gateway mailbox publication slots are always required before execution.
    #[must_use]
    pub const fn required(&self) -> bool {
        true
    }
}

impl GatewayDeclaration {
    /// Validates an installation declaration and returns its normalized identity hash.
    ///
    /// # Errors
    ///
    /// Returns an error when the declaration violates the bounded gateway contract.
    pub fn validate(&self) -> Result<[u8; 32], GatewayError> {
        match self.handler_contract.as_str() {
            HTTP_HANDLER_CONTRACT_V1 if self.service.is_some() => {
                return Err(GatewayError::ServiceConfigurationForbidden);
            }
            HTTP_HANDLER_CONTRACT_V1 => {}
            HTTP_SERVICE_HANDLER_CONTRACT_V1 => {
                let service = self
                    .service
                    .as_ref()
                    .ok_or(GatewayError::ServiceConfigurationRequired)?;
                service.validate()?;
                if !self.mailbox_publication_slots.is_empty() {
                    return Err(GatewayError::ServiceMailboxPublicationForbidden);
                }
            }
            _ => return Err(GatewayError::UnsupportedHandlerContract),
        }
        // Keep the target selector in the same repository-key grammar as
        // gateway names even when callers construct the domain value directly
        // rather than using `agent-config`.
        GatewayName::parse(self.agent_name.clone())?;
        if self.routes.is_empty()
            || self.routes.len() > MAX_ROUTES
            || self.parameters.is_null()
            || !self.parameters.is_object()
        {
            return Err(GatewayError::InvalidDeclaration);
        }
        let mut routes = BTreeSet::new();
        for route in &self.routes {
            if !routes.insert((&route.path, &route.methods)) {
                return Err(GatewayError::DuplicateRoute);
            }
        }
        if self.secret_slots.len() > 32
            || self
                .secret_slots
                .iter()
                .any(|slot| GatewayName::parse(slot.clone()).is_err())
        {
            return Err(GatewayError::InvalidSecretSlot);
        }
        if self.mailbox_publication_slots.len() > MAX_MAILBOX_PUBLICATION_SLOTS {
            return Err(GatewayError::InvalidMailboxPublicationSlot);
        }
        let mut mailbox_publication_slot_keys = BTreeSet::new();
        for slot in &self.mailbox_publication_slots {
            GatewayMailboxPublicationSlot::new(slot.key.clone(), slot.purpose.clone())?;
            if !mailbox_publication_slot_keys.insert(slot.key.clone()) {
                return Err(GatewayError::DuplicateMailboxPublicationSlot);
            }
        }
        let bytes = serde_json::to_vec(self).map_err(|_| GatewayError::InvalidDeclaration)?;
        Ok(Sha256::digest(bytes).into())
    }
}

/// Errors rejecting ambiguous or unsafe declaration data.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum GatewayError {
    /// The gateway name is malformed.
    #[error("invalid gateway name")]
    InvalidName,
    /// The path is ambiguous or unsafe.
    #[error("invalid route path")]
    InvalidRoutePath,
    /// A service readiness or health path is ambiguous or unsafe.
    #[error("invalid service probe path")]
    InvalidServiceProbePath,
    /// A route has no methods.
    #[error("route requires a method")]
    EmptyMethods,
    /// The contract is unsupported.
    #[error("unsupported gateway handler contract")]
    UnsupportedHandlerContract,
    /// The declaration is malformed.
    #[error("invalid gateway declaration")]
    InvalidDeclaration,
    /// A stateless declaration supplied service-only settings.
    #[error("service configuration is forbidden for the stateless gateway contract")]
    ServiceConfigurationForbidden,
    /// A service declaration omitted its required settings.
    #[error("service configuration is required for the service gateway contract")]
    ServiceConfigurationRequired,
    /// The service loopback port is privileged.
    #[error("service loopback port must be between 1024 and 65535")]
    InvalidServicePort,
    /// Service mode has no mailbox-publication sideband in this contract.
    #[error("mailbox publication is forbidden for the service gateway contract")]
    ServiceMailboxPublicationForbidden,
    /// A declaration repeats a route.
    #[error("duplicate gateway route")]
    DuplicateRoute,
    /// A secret-slot name is malformed.
    #[error("invalid gateway secret slot")]
    InvalidSecretSlot,
    /// A mailbox-publication slot is malformed or exceeds its bounded shape.
    #[error("invalid gateway mailbox publication slot")]
    InvalidMailboxPublicationSlot,
    /// A declaration repeats a mailbox-publication slot key.
    #[error("duplicate gateway mailbox publication slot")]
    DuplicateMailboxPublicationSlot,
    /// An inbound secret matcher is malformed.
    #[error("invalid inbound gateway secret rule")]
    InvalidInboundSecretRule,
}
