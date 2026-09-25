//! Pure resolution of UI gateway references to exact release-agent IDs.
//!
//! This module only binds validated source declarations to caller-supplied
//! release identities. It does not create gateway rows or infer identities.

use super::{RepositoryUisConfig, UiApiBinding, UiContent};
use crate::RepositoryGatewaysConfig;
use gateway_domain::{GatewayName, HttpMethod, RoutePath};
use release_domain::ui::{UiKey, UiRoutePath};
use release_domain::{AgentKey, ReleaseAgentId};
use std::{collections::BTreeMap, fmt};

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

/// Resolves UI gateway references to caller-supplied exact release-agent IDs.
///
/// The optional canonical gateway configuration is required whenever a UI has
/// an API or managed-service reference. Static UIs without gateway references
/// resolve successfully with `None`. The source validator remains authoritative
/// for authenticated exposure, service contract, method, and route coverage.
///
/// # Errors
///
/// Returns a redacted, index-only error when source validation fails, supplied
/// agent bindings are ambiguous, or a referenced gateway/agent is unavailable.
pub fn resolve_gateway_uis(
    uis: &RepositoryUisConfig,
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &[ReleaseAgentBinding],
) -> Result<ResolvedGatewayUis, GatewayResolutionError> {
    let diagnostics = super::validate_repository_uis_against_gateways(uis, gateways);
    if !diagnostics.is_empty() {
        return Err(GatewayResolutionError::InvalidManifest {
            diagnostic_count: diagnostics.len(),
        });
    }

    let mut by_key: BTreeMap<AgentKey, (ReleaseAgentId, usize)> = BTreeMap::new();
    let mut by_id = BTreeMap::new();
    for (candidate_index, agent) in agents.iter().enumerate() {
        if let Some((_, first_index)) = by_key.insert(
            agent.agent_key.clone(),
            (agent.release_agent_id, candidate_index),
        ) {
            return Err(GatewayResolutionError::DuplicateAgentKey {
                candidate_index,
                first_index,
            });
        }
        if let Some(first_index) = by_id.insert(agent.release_agent_id, candidate_index) {
            return Err(GatewayResolutionError::DuplicateAgentId {
                candidate_index,
                first_index,
            });
        }
    }

    let mut resolved = Vec::with_capacity(uis.uis.len());
    for (ui_index, ui) in uis.uis.iter().enumerate() {
        let managed_service = match &ui.content {
            UiContent::ManagedService {
                gateway_name,
                route,
                entrypoint,
            } => Some(resolve_managed_service(
                gateways,
                &by_key,
                ui_index,
                gateway_name,
                route,
                entrypoint,
            )?),
            UiContent::Static { .. } => None,
        };
        let api_bindings = ui
            .apis
            .iter()
            .enumerate()
            .map(|(api_index, api)| resolve_api(gateways, &by_key, ui_index, api_index, api))
            .collect::<Result<Vec<_>, _>>()?;
        resolved.push(ResolvedGatewayUi {
            key: ui.key.clone(),
            managed_service,
            apis: api_bindings,
        });
    }
    Ok(ResolvedGatewayUis { uis: resolved })
}

fn resolve_managed_service(
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    ui_index: usize,
    gateway_name: &GatewayName,
    route: &RoutePath,
    entrypoint: &UiRoutePath,
) -> Result<ResolvedManagedService, GatewayResolutionError> {
    let gateway =
        find_gateway(gateways, gateway_name).ok_or(GatewayResolutionError::MissingGateway {
            ui_index,
            reference_kind: GatewayReferenceKind::ManagedService,
            reference_index: 0,
        })?;
    let agent_id = resolve_agent_id(
        agents,
        &gateway.agent_name,
        ui_index,
        GatewayReferenceKind::ManagedService,
        0,
    )?;
    Ok(ResolvedManagedService {
        gateway_name: gateway_name.clone(),
        release_agent_id: agent_id,
        route: route.clone(),
        entrypoint: entrypoint.clone(),
    })
}

fn resolve_api(
    gateways: Option<&RepositoryGatewaysConfig>,
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    ui_index: usize,
    api_index: usize,
    api: &UiApiBinding,
) -> Result<ResolvedApiBinding, GatewayResolutionError> {
    let gateway = find_gateway(gateways, &api.gateway_name).ok_or(
        GatewayResolutionError::MissingGateway {
            ui_index,
            reference_kind: GatewayReferenceKind::Api,
            reference_index: api_index,
        },
    )?;
    let agent_id = resolve_agent_id(
        agents,
        &gateway.agent_name,
        ui_index,
        GatewayReferenceKind::Api,
        api_index,
    )?;
    Ok(ResolvedApiBinding {
        key: api.key.clone(),
        gateway_name: api.gateway_name.clone(),
        release_agent_id: agent_id,
        method: api.method,
        route: api.route.clone(),
    })
}

fn find_gateway<'a>(
    gateways: Option<&'a RepositoryGatewaysConfig>,
    requested: &GatewayName,
) -> Option<&'a crate::RepositoryGatewayConfig> {
    gateways?.gateways.iter().find(|gateway| {
        GatewayName::parse(gateway.name.clone()).is_ok_and(|name| name == *requested)
    })
}

fn resolve_agent_id(
    agents: &BTreeMap<AgentKey, (ReleaseAgentId, usize)>,
    agent_name: &str,
    ui_index: usize,
    reference_kind: GatewayReferenceKind,
    reference_index: usize,
) -> Result<ReleaseAgentId, GatewayResolutionError> {
    let Ok(agent_key) = AgentKey::parse(agent_name.to_owned()) else {
        return Err(GatewayResolutionError::MissingAgent {
            ui_index,
            reference_kind,
            reference_index,
        });
    };
    let Some(&(release_agent_id, _candidate_index)) = agents.get(&agent_key) else {
        return Err(GatewayResolutionError::MissingAgent {
            ui_index,
            reference_kind,
            reference_index,
        });
    };
    Ok(release_agent_id)
}

#[cfg(test)]
mod tests {
    use super::{
        GatewayReferenceKind, GatewayResolutionError, ReleaseAgentBinding, resolve_gateway_uis,
    };
    use crate::{parse_repository_gateways, parse_repository_uis};
    use release_domain::{AgentKey, ReleaseAgentId};
    use uuid::Uuid;

    const UI: &str = r#"
version = 1

[[uis]]
key = "assistant"
scope = "project"
label = "Assistant"
icon = "chat"
presentation = "iframe"
route_base = "assistant"
ui_kit_version = 1
cache = "no_store"

[[uis.apis]]
key = "service-api"
gateway_name = "ui-service"
method = "GET"
route = "/service/api"

[uis.content]
kind = "managed_service"
gateway_name = "ui-service"
route = "/service/ui"
entrypoint = "index.html"
"#;

    const GATEWAYS: &str = r#"
version = 1

[[gateways]]
name = "ui-service"
agent_name = "ui-service-agent"
handler_contract = "http.service.v1"
exposure = "heph_authenticated"

[gateways.service]
loopback_port = 8080
readiness_path = "/ready"
health_path = "/health"

[[gateways.routes]]
path = "/service"
methods = ["GET"]
"#;

    fn id(value: u128) -> ReleaseAgentId {
        ReleaseAgentId::from_uuid(Uuid::from_u128(value))
    }

    fn binding(key: &str, value: u128) -> ReleaseAgentBinding {
        ReleaseAgentBinding {
            agent_key: AgentKey::parse(key).expect("agent key"),
            release_agent_id: id(value),
        }
    }

    fn parsed() -> (
        crate::ui::RepositoryUisConfig,
        crate::RepositoryGatewaysConfig,
    ) {
        (
            parse_repository_uis(UI.as_bytes())
                .config
                .expect("valid UI"),
            parse_repository_gateways(GATEWAYS.as_bytes())
                .config
                .expect("valid gateways"),
        )
    }

    #[test]
    fn retains_exact_agent_id_for_managed_and_api_bindings() {
        let (uis, gateways) = parsed();
        let result = resolve_gateway_uis(&uis, Some(&gateways), &[binding("ui-service-agent", 1)])
            .expect("gateway bindings");
        assert_eq!(
            result.uis[0]
                .managed_service
                .as_ref()
                .expect("managed service")
                .release_agent_id,
            id(1)
        );
        assert_eq!(result.uis[0].apis[0].release_agent_id, id(1));
    }

    #[test]
    fn missing_agent_is_rejected_without_gateway_name_echo() {
        let (uis, gateways) = parsed();
        let error = resolve_gateway_uis(&uis, Some(&gateways), &[binding("other-agent", 1)])
            .expect_err("missing agent");
        assert_eq!(
            error,
            GatewayResolutionError::MissingAgent {
                ui_index: 0,
                reference_kind: GatewayReferenceKind::ManagedService,
                reference_index: 0,
            }
        );
        assert!(!error.to_string().contains("ui-service"));
        assert!(!error.to_string().contains("ui-service-agent"));
    }

    #[test]
    fn static_ui_needs_no_gateway_manifest() {
        let uis = parse_repository_uis(
            "version = 1\n\n[[uis]]\nkey = \"docs\"\nscope = \"global\"\nlabel = \"Docs\"\nicon = \"book\"\npresentation = \"iframe\"\nroute_base = \"docs\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n".as_bytes(),
        )
        .config
        .expect("valid static UI");
        let result = resolve_gateway_uis(&uis, None, &[]).expect("static UI");
        assert!(result.uis[0].managed_service.is_none());
        assert!(result.uis[0].apis.is_empty());
    }

    #[test]
    fn one_agent_can_back_two_distinct_gateway_names() {
        let ui = parse_repository_uis(
            "version = 1\n\n[[uis]]\nkey = \"api-ui\"\nscope = \"global\"\nlabel = \"API UI\"\nicon = \"app\"\npresentation = \"iframe\"\nroute_base = \"api-ui\"\nui_kit_version = 1\ncache = \"no_store\"\n\n[[uis.apis]]\nkey = \"one\"\ngateway_name = \"one\"\nmethod = \"GET\"\nroute = \"/one\"\n\n[[uis.apis]]\nkey = \"two\"\ngateway_name = \"two\"\nmethod = \"GET\"\nroute = \"/two\"\n\n[uis.content]\nkind = \"static\"\nentrypoint = \"index.html\"\n\n[[uis.content.files]]\nroute = \"index.html\"\nartifact = \"dist/index.html\"\nmedia_type = \"text/html\"\n".as_bytes(),
        )
        .config
        .expect("valid UI");
        let gateways = parse_repository_gateways(
            "version = 1\n\n[[gateways]]\nname = \"one\"\nagent_name = \"shared-agent\"\nhandler_contract = \"http.v1\"\nexposure = \"heph_authenticated\"\n\n[[gateways.routes]]\npath = \"/one\"\nmethods = [\"GET\"]\n\n[[gateways]]\nname = \"two\"\nagent_name = \"shared-agent\"\nhandler_contract = \"http.v1\"\nexposure = \"heph_authenticated\"\n\n[[gateways.routes]]\npath = \"/two\"\nmethods = [\"GET\"]\n".as_bytes(),
        )
        .config
        .expect("valid gateways");
        let result = resolve_gateway_uis(&ui, Some(&gateways), &[binding("shared-agent", 3)])
            .expect("shared agent bindings");
        assert_eq!(result.uis[0].apis[0].release_agent_id, id(3));
        assert_eq!(result.uis[0].apis[1].release_agent_id, id(3));
    }

    #[test]
    fn duplicate_agent_keys_and_ids_are_rejected() {
        let (uis, gateways) = parsed();
        let error = resolve_gateway_uis(
            &uis,
            Some(&gateways),
            &[
                binding("ui-service-agent", 1),
                binding("ui-service-agent", 2),
            ],
        )
        .expect_err("duplicate key");
        assert!(matches!(
            error,
            GatewayResolutionError::DuplicateAgentKey { .. }
        ));

        let error = resolve_gateway_uis(
            &uis,
            Some(&gateways),
            &[binding("ui-service-agent", 1), binding("other-agent", 1)],
        )
        .expect_err("duplicate ID");
        assert!(matches!(
            error,
            GatewayResolutionError::DuplicateAgentId { .. }
        ));
    }

    #[test]
    fn public_or_uncovered_routes_fail_before_identity_resolution() {
        let (uis, _) = parsed();
        let public_gateways = parse_repository_gateways(
            GATEWAYS
                .replace("exposure = \"heph_authenticated\"", "exposure = \"public\"")
                .as_bytes(),
        )
        .config
        .expect("gateway syntax");
        let error = resolve_gateway_uis(
            &uis,
            Some(&public_gateways),
            &[binding("ui-service-agent", 1)],
        )
        .expect_err("public gateway");
        assert!(matches!(
            error,
            GatewayResolutionError::InvalidManifest { .. }
        ));

        let uncovered_gateways = parse_repository_gateways(
            GATEWAYS
                .replace("path = \"/service\"", "path = \"/other\"")
                .as_bytes(),
        )
        .config
        .expect("gateway syntax");
        let error = resolve_gateway_uis(
            &uis,
            Some(&uncovered_gateways),
            &[binding("ui-service-agent", 1)],
        )
        .expect_err("uncovered route");
        assert!(matches!(
            error,
            GatewayResolutionError::InvalidManifest { .. }
        ));
    }
}
