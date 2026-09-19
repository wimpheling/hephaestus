//! Canonical, provider-neutral gateway declarations and bounded HTTP values.
//!
//! This crate deliberately contains no listener, provider, database, or VM
//! code. It is the exact shared vocabulary for the control plane and edge.

use async_trait::async_trait;
use capability_domain::{CapabilityOperation, CapabilityResourceKind, CapabilitySlotKey};
use http::{HeaderName, HeaderValue};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, fmt, str::FromStr};
use thiserror::Error;
use uuid::Uuid;

/// One host-only exact inbound secret substitution rule.
pub struct InboundGatewaySecretRule {
    /// Header whose exact inbound value is matched.
    pub header: HeaderName,
    /// Decrypted value held only until dispatch completes.
    pub expected: Vec<u8>,
    /// Non-secret value delivered to the guest.
    pub placeholder: HeaderValue,
}

impl InboundGatewaySecretRule {
    /// Creates a non-empty exact substitution rule.
    ///
    /// # Errors
    ///
    /// Returns [`GatewayError::InvalidInboundSecretRule`] when either the
    /// expected value or safe replacement is empty.
    pub fn new(
        header: HeaderName,
        expected: Vec<u8>,
        placeholder: HeaderValue,
    ) -> Result<Self, GatewayError> {
        if expected.is_empty() || placeholder.as_bytes().is_empty() {
            return Err(GatewayError::InvalidInboundSecretRule);
        }
        Ok(Self {
            header,
            expected,
            placeholder,
        })
    }
}

impl Drop for InboundGatewaySecretRule {
    fn drop(&mut self) {
        self.expected.fill(0);
    }
}

/// Gateway-owned port for resolving already-authorized inbound secret rules.
#[async_trait]
pub trait GatewayInboundSecretResolver: Send + Sync {
    /// Returns rules for one accepted invocation and exact immutable route.
    async fn rules_for_invocation(
        &self,
        invocation_id: Uuid,
        route_id: Uuid,
        gateway_revision_id: Uuid,
    ) -> Result<Vec<InboundGatewaySecretRule>, GatewayError>;
}

/// Maximum bytes accepted in one canonical request or response body.
pub const MAX_BODY_BYTES: u32 = 1_048_576;
/// Maximum routes declared by one gateway revision.
pub const MAX_ROUTES: usize = 32;
/// Maximum mailbox-publication slots declared by one gateway revision.
pub const MAX_MAILBOX_PUBLICATION_SLOTS: usize = 32;
/// The bounded one-request gateway HTTP handler contract.
pub const HTTP_HANDLER_CONTRACT_V1: &str = "http.v1";
/// The explicitly declared long-lived gateway HTTP service contract.
pub const HTTP_SERVICE_HANDLER_CONTRACT_V1: &str = "http.service.v1";

macro_rules! identifier {
    ($name:ident, $docs:literal) => {
        #[doc = $docs]
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Uuid);
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl $name {
            /// Creates a new random identifier.
            #[must_use]
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            /// Reconstitutes an identifier from its UUID.
            #[must_use]
            pub const fn from_uuid(value: Uuid) -> Self {
                Self(value)
            }
            /// Returns the underlying UUID.
            #[must_use]
            pub const fn as_uuid(self) -> Uuid {
                self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
        impl FromStr for $name {
            type Err = uuid::Error;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Uuid::parse_str(s).map(Self)
            }
        }
    };
}
identifier!(GatewayId, "A durable project-owned gateway identity.");
identifier!(
    GatewayRevisionId,
    "An immutable gateway declaration revision."
);
identifier!(GatewayRouteId, "A durable declared gateway route.");
identifier!(
    GatewayTargetId,
    "A stable target selected by a declared route."
);
identifier!(
    GatewayReconciliationId,
    "A provider reconciliation attempt."
);

/// A stable repository-scoped gateway name.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct GatewayName(String);
impl GatewayName {
    /// Parses a bounded lowercase name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is not a bounded lowercase identifier.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(1..=64).contains(&value.len())
            || !value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-')
        {
            return Err(GatewayError::InvalidName);
        }
        Ok(Self(value))
    }
    /// Returns its normalized value.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for GatewayName {
    type Error = GatewayError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(v)
    }
}
impl From<GatewayName> for String {
    fn from(value: GatewayName) -> Self {
        value.0
    }
}

/// Public exposure policy. Authenticated forwarding is deliberately reserved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    /// Public route.
    Public,
    /// Reserved future policy.
    HephAuthenticated,
}

/// Lifecycle state of a durable gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GatewayLifecycle {
    /// Accepts future invocations.
    Enabled,
    /// Retained but rejects new invocations.
    Paused,
    /// Retained tombstone.
    Removed,
}

/// Bounded method accepted by the canonical contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    /// GET.
    Get,
    /// POST.
    Post,
    /// PUT.
    Put,
    /// PATCH.
    Patch,
    /// DELETE.
    Delete,
    /// HEAD.
    Head,
    /// OPTIONS.
    Options,
}

/// A normalized, non-wildcard path prefix under the reserved gateway namespace.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct RoutePath(String);
impl RoutePath {
    /// Parses a canonical absolute path without encoding, dot segments, or trailing slash.
    ///
    /// # Errors
    ///
    /// Returns an error when the path is ambiguous or outside the route grammar.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(2..=512).contains(&value.len())
            || !value.starts_with('/')
            || value.ends_with('/')
            || value.contains(['?', '#', '%', '*'])
            || value.split('/').any(|part| part == "." || part == "..")
        {
            return Err(GatewayError::InvalidRoutePath);
        }
        Ok(Self(value))
    }
    /// Returns the canonical path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for RoutePath {
    type Error = GatewayError;
    fn try_from(v: String) -> Result<Self, Self::Error> {
        Self::parse(v)
    }
}
impl From<RoutePath> for String {
    fn from(value: RoutePath) -> Self {
        value.0
    }
}

/// A strict origin-form path used for service readiness and health probes.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ServiceProbePath(String);

impl ServiceProbePath {
    /// Parses a bounded origin-form path without ambiguous URL syntax.
    ///
    /// # Errors
    ///
    /// Returns an error for query strings, fragments, encoding, control or
    /// whitespace characters, backslashes, repeated separators, dot segments,
    /// or trailing slashes other than the root path.
    pub fn parse(value: impl Into<String>) -> Result<Self, GatewayError> {
        let value = value.into();
        if !(1..=512).contains(&value.len())
            || !value.starts_with('/')
            || (value.len() > 1 && value.ends_with('/'))
            || value.contains(['?', '#', '%', '*', '\\'])
            || value.contains("//")
            || value
                .chars()
                .any(|character| character.is_control() || character.is_whitespace())
            || value.split('/').any(|part| part == "." || part == "..")
        {
            return Err(GatewayError::InvalidServiceProbePath);
        }
        Ok(Self(value))
    }

    /// Returns the canonical origin-form path.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ServiceProbePath {
    type Error = GatewayError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::parse(value)
    }
}

impl From<ServiceProbePath> for String {
    fn from(value: ServiceProbePath) -> Self {
        value.0
    }
}

/// One bounded, non-live route request in a gateway revision.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RouteIntent {
    /// Canonical path prefix.
    pub path: RoutePath,
    /// Allowed methods.
    pub methods: BTreeSet<HttpMethod>,
}
impl RouteIntent {
    /// Validates the bounded route intent.
    ///
    /// # Errors
    ///
    /// Returns an error when no HTTP method is selected.
    pub fn new(
        path: RoutePath,
        methods: impl IntoIterator<Item = HttpMethod>,
    ) -> Result<Self, GatewayError> {
        let methods = methods.into_iter().collect::<BTreeSet<_>>();
        if methods.is_empty() {
            return Err(GatewayError::EmptyMethods);
        }
        Ok(Self { path, methods })
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct LegacyDeclaration<'a> {
        name: &'a GatewayName,
        agent_name: &'a str,
        handler_contract: &'a str,
        exposure: Exposure,
        routes: &'a Vec<RouteIntent>,
        parameters: &'a serde_json::Value,
        secret_slots: &'a Vec<String>,
        mailbox_publication_slots: &'a Vec<GatewayMailboxPublicationSlot>,
    }

    #[test]
    fn rejects_ambiguous_paths() {
        assert!(RoutePath::parse("/a/../b").is_err());
        assert!(RoutePath::parse("/a%2fb").is_err());
    }

    #[test]
    fn validates_service_configuration_and_contract_pairing() {
        let service = GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/ready").unwrap(),
            ServiceProbePath::parse("/health").unwrap(),
        )
        .unwrap();
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("service").unwrap(),
            agent_name: String::from("service-handler"),
            handler_contract: HTTP_SERVICE_HANDLER_CONTRACT_V1.into(),
            service: Some(service),
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/service").unwrap(), [HttpMethod::Get]).unwrap(),
            ],
            parameters: serde_json::json!({}),
            secret_slots: vec![],
            mailbox_publication_slots: vec![],
        };
        assert!(declaration.validate().is_ok());

        let mut missing_service = declaration.clone();
        missing_service.service = None;
        assert_eq!(
            missing_service.validate(),
            Err(GatewayError::ServiceConfigurationRequired)
        );

        let mut service_with_mailbox = declaration.clone();
        service_with_mailbox.mailbox_publication_slots = vec![
            GatewayMailboxPublicationSlot::new(
                CapabilitySlotKey::parse("deliver").unwrap(),
                "Deliver an accepted event.",
            )
            .unwrap(),
        ];
        assert_eq!(
            service_with_mailbox.validate(),
            Err(GatewayError::ServiceMailboxPublicationForbidden)
        );

        let mut stateless_with_service = declaration;
        stateless_with_service.handler_contract = HTTP_HANDLER_CONTRACT_V1.into();
        assert_eq!(
            stateless_with_service.validate(),
            Err(GatewayError::ServiceConfigurationForbidden)
        );
    }

    #[test]
    fn rejects_privileged_service_ports_and_ambiguous_service_paths() {
        assert_eq!(
            GatewayServiceConfig::new(
                1023,
                ServiceProbePath::parse("/ready").unwrap(),
                ServiceProbePath::parse("/health").unwrap(),
            ),
            Err(GatewayError::InvalidServicePort)
        );
        assert!(ServiceProbePath::parse("/").is_ok());
        for invalid in [
            "/ready?probe",
            "/ready%2fprobe",
            "/ready//probe",
            "/ready/",
            "/ready/../probe",
            "/ready\\probe",
            "/ready probe",
            "/ready\nprobe",
        ] {
            assert!(ServiceProbePath::parse(invalid).is_err(), "{invalid:?}");
        }
        let invalid_json =
            r#"{"loopback_port":8080,"readiness_path":"/ready?probe","health_path":"/health"}"#;
        assert!(serde_json::from_str::<GatewayServiceConfig>(invalid_json).is_err());
        let root_json = r#"{"loopback_port":8080,"readiness_path":"/","health_path":"/health"}"#;
        assert!(serde_json::from_str::<GatewayServiceConfig>(root_json).is_ok());
    }

    #[test]
    fn stateless_service_field_is_omitted_from_legacy_hash_serialization() {
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("telegram").unwrap(),
            agent_name: String::from("telegram-handler"),
            handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
            service: None,
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post])
                    .unwrap(),
            ],
            parameters: serde_json::json!({"enabled": true}),
            secret_slots: vec!["telegram_secret".into()],
            mailbox_publication_slots: vec![],
        };
        let expected: [u8; 32] = Sha256::digest(
            serde_json::to_vec(&LegacyDeclaration {
                name: &declaration.name,
                agent_name: &declaration.agent_name,
                handler_contract: &declaration.handler_contract,
                exposure: declaration.exposure,
                routes: &declaration.routes,
                parameters: &declaration.parameters,
                secret_slots: &declaration.secret_slots,
                mailbox_publication_slots: &declaration.mailbox_publication_slots,
            })
            .unwrap(),
        )
        .into();
        assert_eq!(declaration.validate().unwrap(), expected);
        assert!(
            !serde_json::to_string(&declaration)
                .unwrap()
                .contains("service")
        );
    }

    #[test]
    fn service_log_capture_default_preserves_legacy_hash_and_opt_in_changes_it() {
        let service = GatewayServiceConfig::new(
            8080,
            ServiceProbePath::parse("/ready").unwrap(),
            ServiceProbePath::parse("/health").unwrap(),
        )
        .unwrap();
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("service").unwrap(),
            agent_name: String::from("service-handler"),
            handler_contract: HTTP_SERVICE_HANDLER_CONTRACT_V1.into(),
            service: Some(service),
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/service").unwrap(), [HttpMethod::Get]).unwrap(),
            ],
            parameters: serde_json::json!({}),
            secret_slots: vec![],
            mailbox_publication_slots: vec![],
        };
        let omitted_json = serde_json::to_vec(&declaration).unwrap();
        let omitted_hash = declaration.validate().unwrap();
        let frozen_json = br#"{"name":"service","agent_name":"service-handler","handler_contract":"http.service.v1","service":{"loopback_port":8080,"readiness_path":"/ready","health_path":"/health"},"exposure":"public","routes":[{"path":"/service","methods":["GET"]}],"parameters":{},"secret_slots":[],"mailbox_publication_slots":[]}"#;
        let frozen_hash: [u8; 32] = [
            0x76, 0x72, 0xda, 0x47, 0x84, 0x24, 0xac, 0x3c, 0xe3, 0x3b, 0x1b, 0xc1, 0x99, 0x51,
            0xa9, 0x49, 0x70, 0x88, 0x43, 0xf6, 0x38, 0x2d, 0x42, 0x74, 0xcf, 0xe9, 0x19, 0xe9,
            0x1d, 0xcf, 0xe1, 0xc8,
        ];
        assert_eq!(omitted_json, frozen_json);
        assert_eq!(omitted_hash, frozen_hash);

        let mut explicit_disabled = declaration.clone();
        explicit_disabled.service.as_mut().unwrap().log_capture_mode =
            ServiceLogCaptureMode::Disabled;
        assert_eq!(omitted_hash, explicit_disabled.validate().unwrap());
        assert_eq!(
            omitted_json,
            serde_json::to_vec(&explicit_disabled).unwrap()
        );
        assert!(
            !String::from_utf8(omitted_json)
                .unwrap()
                .contains("log_capture_mode")
        );

        let mut application = declaration;
        application.service.as_mut().unwrap().log_capture_mode = ServiceLogCaptureMode::Application;
        assert_ne!(omitted_hash, application.validate().unwrap());
        assert!(
            serde_json::to_string(&application)
                .unwrap()
                .contains("application")
        );
    }

    #[test]
    fn rejects_unknown_service_log_capture_mode() {
        let source = r#"{
            "loopback_port": 8080,
            "readiness_path": "/ready",
            "health_path": "/health",
            "log_capture_mode": "future"
        }"#;
        assert!(serde_json::from_str::<GatewayServiceConfig>(source).is_err());
    }

    #[test]
    fn validates_a_gateway() {
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("telegram").unwrap(),
            agent_name: String::from("telegram-handler"),
            handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
            service: None,
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post])
                    .unwrap(),
            ],
            parameters: serde_json::json!({}),
            secret_slots: vec!["telegram_secret".into()],
            mailbox_publication_slots: vec![
                GatewayMailboxPublicationSlot::new(
                    CapabilitySlotKey::parse("agent_mailbox").unwrap(),
                    "Deliver accepted gateway updates to the agent.",
                )
                .unwrap(),
            ],
        };
        assert!(declaration.validate().is_ok());
        let slot = &declaration.mailbox_publication_slots[0];
        assert_eq!(slot.resource_kind(), CapabilityResourceKind::Mailbox);
        assert_eq!(slot.required_operation(), CapabilityOperation::Publish);
        assert!(slot.required());
    }

    #[test]
    fn rejects_duplicate_mailbox_publication_slots() {
        let slot = GatewayMailboxPublicationSlot::new(
            CapabilitySlotKey::parse("agent_mailbox").unwrap(),
            "Deliver accepted gateway updates to the agent.",
        )
        .unwrap();
        let declaration = GatewayDeclaration {
            name: GatewayName::parse("telegram").unwrap(),
            agent_name: String::from("telegram-handler"),
            handler_contract: HTTP_HANDLER_CONTRACT_V1.into(),
            service: None,
            exposure: Exposure::Public,
            routes: vec![
                RouteIntent::new(RoutePath::parse("/telegram").unwrap(), [HttpMethod::Post])
                    .unwrap(),
            ],
            parameters: serde_json::json!({}),
            secret_slots: vec![],
            mailbox_publication_slots: vec![slot.clone(), slot],
        };
        assert_eq!(
            declaration.validate(),
            Err(GatewayError::DuplicateMailboxPublicationSlot)
        );
    }
}
