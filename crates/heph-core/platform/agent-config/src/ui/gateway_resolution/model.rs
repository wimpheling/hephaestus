//! Public result types and redacted errors for UI gateway resolution.

use gateway_domain::{GatewayName, HttpMethod, RoutePath};
use release_domain::ui::{UiKey, UiRoutePath};
use release_domain::{AgentKey, ReleaseAgentId};
use std::fmt;

/// One exact release agent available to gateway resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAgentBinding {
    /// Existing exported agent key.
    pub agent_key: AgentKey,
    /// Exact immutable ID from the release being assembled.
    pub release_agent_id: ReleaseAgentId,
}

/// One managed-service UI binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedManagedService {
    /// Referenced repository gateway name.
    pub gateway_name: GatewayName,
    /// Exact agent ID selected by that gateway declaration.
    pub release_agent_id: ReleaseAgentId,
    /// Absolute gateway route.
    pub route: RoutePath,
    /// UI-relative entrypoint.
    pub entrypoint: UiRoutePath,
}

/// One UI API binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedApiBinding {
    /// UI-local API key.
    pub key: UiKey,
    /// Referenced repository gateway name.
    pub gateway_name: GatewayName,
    /// Exact agent ID selected by that gateway declaration.
    pub release_agent_id: ReleaseAgentId,
    /// Authorized HTTP method.
    pub method: HttpMethod,
    /// Absolute gateway route.
    pub route: RoutePath,
}

/// Resolved gateway bindings for one UI key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGatewayUi {
    /// Stable UI key.
    pub key: UiKey,
    /// Optional managed-service binding.
    pub managed_service: Option<ResolvedManagedService>,
    /// API bindings in manifest order.
    pub apis: Vec<ResolvedApiBinding>,
}

/// Gateway bindings for every UI declaration, including static UIs with no
/// gateway references.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedGatewayUis {
    /// UI declarations in manifest order.
    pub uis: Vec<ResolvedGatewayUi>,
}

/// Whether an indexed reference came from managed content or an API entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayReferenceKind {
    /// Managed-service content reference.
    ManagedService,
    /// UI API reference.
    Api,
}

/// Redacted failure from gateway resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GatewayResolutionError {
    /// UI/gateway source validation failed.
    InvalidManifest {
        /// Number of validation diagnostics.
        diagnostic_count: usize,
    },
    /// Two supplied release agents use one key.
    DuplicateAgentKey {
        /// Index of the later binding.
        candidate_index: usize,
        /// Index of the first binding with this key.
        first_index: usize,
    },
    /// Two supplied release agents use one ID.
    DuplicateAgentId {
        /// Index of the later binding.
        candidate_index: usize,
        /// Index of the first binding with this ID.
        first_index: usize,
    },
    /// A validated UI reference could not find its gateway declaration.
    MissingGateway {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Kind of UI reference.
        reference_kind: GatewayReferenceKind,
        /// Index of the reference within its UI collection.
        reference_index: usize,
    },
    /// A gateway declaration points at no supplied release agent.
    MissingAgent {
        /// Index of the UI declaration.
        ui_index: usize,
        /// Kind of UI reference.
        reference_kind: GatewayReferenceKind,
        /// Index of the reference within its UI collection.
        reference_index: usize,
    },
}

impl fmt::Display for GatewayResolutionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidManifest { diagnostic_count } => write!(
                formatter,
                "UI gateway validation failed ({diagnostic_count} diagnostics)"
            ),
            Self::DuplicateAgentKey {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate release-agent key at agents[{candidate_index}] (first at agents[{first_index}])"
            ),
            Self::DuplicateAgentId {
                candidate_index,
                first_index,
            } => write!(
                formatter,
                "duplicate release-agent ID at agents[{candidate_index}] (first at agents[{first_index}])"
            ),
            Self::MissingGateway {
                ui_index,
                reference_kind,
                reference_index,
            } => write!(
                formatter,
                "gateway declaration is unavailable at uis[{ui_index}].{reference_kind}[{reference_index}]"
            ),
            Self::MissingAgent {
                ui_index,
                reference_kind,
                reference_index,
            } => write!(
                formatter,
                "gateway release agent is unavailable at uis[{ui_index}].{reference_kind}[{reference_index}]"
            ),
        }
    }
}

impl fmt::Display for GatewayReferenceKind {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::ManagedService => "content",
            Self::Api => "apis",
        })
    }
}

impl std::error::Error for GatewayResolutionError {}
